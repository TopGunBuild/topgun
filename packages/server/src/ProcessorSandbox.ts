import {
  EntryProcessorDef,
  EntryProcessorResult,
  validateProcessorCode,
} from '@topgunbuild/core';
import { logger } from './utils/logger';

// Types for isolated-vm (optional dependency)
interface IsolatedVmIsolate {
  isDisposed: boolean;
  createContext(): Promise<IsolatedVmContext>;
  compileScript(code: string): Promise<IsolatedVmScript>;
  dispose(): void;
}

interface IsolatedVmContext {
  global: IsolatedVmReference;
  eval(code: string): Promise<void>;
}

interface IsolatedVmScript {
  run(context: IsolatedVmContext, options: { timeout: number }): Promise<unknown>;
}

interface IsolatedVmReference {
  set(key: string, value: unknown): Promise<void>;
  derefInto(): unknown;
}

interface IsolatedVmModule {
  Isolate: new (options: { memoryLimit: number }) => IsolatedVmIsolate;
}

// Try to import isolated-vm, fall back to unsafe VM for development
let ivm: IsolatedVmModule | null = null;
try {
  ivm = require('isolated-vm');
} catch {
  const isProduction = process.env.NODE_ENV === 'production';
  if (isProduction) {
    logger.error(
      'SECURITY WARNING: isolated-vm not available in production! ' +
      'Entry processors will run in less secure fallback mode. ' +
      'Install isolated-vm for production environments: pnpm add isolated-vm'
    );
  } else {
    logger.warn('isolated-vm not available, falling back to less secure VM');
  }
}

/**
 * Configuration for the processor sandbox.
 */
export interface ProcessorSandboxConfig {
  /** Memory limit in MB per isolate */
  memoryLimitMb: number;

  /** Execution timeout in milliseconds */
  timeoutMs: number;

  /** Maximum number of cached isolates */
  maxCachedIsolates: number;

  /** Enable strict code validation */
  strictValidation: boolean;
}

/**
 * Default sandbox configuration.
 */
export const DEFAULT_SANDBOX_CONFIG: ProcessorSandboxConfig = {
  memoryLimitMb: 8,
  timeoutMs: 100,
  maxCachedIsolates: 100,
  strictValidation: true,
};

/**
 * Sandbox for executing entry processor code securely.
 *
 * Uses isolated-vm for production environments with:
 * - Memory limits to prevent memory bombs
 * - CPU limits via timeout to prevent infinite loops
 * - No I/O access (no require, fs, net, etc.)
 * - Minimal exposed globals (only value, key, args)
 *
 * Falls back to Node.js vm module for development/testing
 * when isolated-vm is not available.
 */
export class ProcessorSandbox {
  private config: ProcessorSandboxConfig;
  private isolateCache: Map<string, IsolatedVmIsolate> = new Map();
  private scriptCache: Map<string, IsolatedVmScript> = new Map();
  private fallbackScriptCache: Map<string, (value: unknown, key: string, args: unknown) => unknown> = new Map();
  private disposed = false;

  constructor(config: Partial<ProcessorSandboxConfig> = {}) {
    this.config = { ...DEFAULT_SANDBOX_CONFIG, ...config };
  }

  /**
   * Execute an entry processor in the sandbox.
   *
   * @param processor The processor definition (name, code, args)
   * @param value The current value for the key (or undefined)
   * @param key The key being processed
   * @returns Result containing success status, result, and new value
   */
  async execute<V, R>(
    processor: EntryProcessorDef<V, R>,
    value: V | undefined,
    key: string,
  ): Promise<EntryProcessorResult<R>> {
    if (this.disposed) {
      return {
        success: false,
        error: 'Sandbox has been disposed',
      };
    }

    // Validate code if strict validation is enabled
    if (this.config.strictValidation) {
      const validation = validateProcessorCode(processor.code);
      if (!validation.valid) {
        return {
          success: false,
          error: validation.error,
        };
      }
    }

    // Use isolated-vm if available, otherwise fall back
    if (ivm) {
      return this.executeInIsolate(processor, value, key);
    } else {
      return this.executeInFallback(processor, value, key);
    }
  }

  /**
   * Execute processor in isolated-vm (secure production mode).
   */
  private async executeInIsolate<V, R>(
    processor: EntryProcessorDef<V, R>,
    value: V | undefined,
    key: string,
  ): Promise<EntryProcessorResult<R>> {
    if (!ivm) {
      return { success: false, error: 'isolated-vm not available' };
    }

    const isolate = this.getOrCreateIsolate(processor.name);

    try {
      const context = await isolate.createContext();
      const jail = context.global;

      // Set up minimal environment
      await jail.set('global', jail.derefInto());

      // Set input values as JSON strings to avoid reference issues
      await context.eval(`
        var value = ${JSON.stringify(value)};
        var key = ${JSON.stringify(key)};
        var args = ${JSON.stringify(processor.args)};
      `);

      // Wrap user code in a function
      const wrappedCode = `
        (function() {
          ${processor.code}
        })()
      `;

      // Get or compile script
      const script = await this.getOrCompileScript(
        processor.name,
        wrappedCode,
        isolate,
      );

      // Execute with timeout
      const result = await script.run(context, {
        timeout: this.config.timeoutMs,
      });

      // Validate result format
      const parsed = result as { value: V | undefined; result?: R };

      if (typeof parsed !== 'object' || parsed === null) {
        return {
          success: false,
          error: 'Processor must return { value, result? } object',
        };
      }

      return {
        success: true,
        result: parsed.result,
        newValue: parsed.value,
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);

      // Detect timeout errors
      if (message.includes('Script execution timed out')) {
        return {
          success: false,
          error: 'Processor execution timed out',
        };
      }

      return {
        success: false,
        error: message,
      };
    }
  }

  /**
   * Execute processor in fallback VM (less secure, for development).
   */
  private async executeInFallback<V, R>(
    processor: EntryProcessorDef<V, R>,
    value: V | undefined,
    key: string,
  ): Promise<EntryProcessorResult<R>> {
    try {
      // Skip caching for resolvers - they embed context in the code string
      const isResolver = processor.name.startsWith('resolver:');
      let fn = isResolver ? undefined : this.fallbackScriptCache.get(processor.name);

      if (!fn) {
        // Create function from code
        const wrappedCode = `
          return (function(value, key, args) {
            ${processor.code}
          })
        `;
        fn = new Function(wrappedCode)() as (value: unknown, key: string, args: unknown) => unknown;
        if (!isResolver) {
          this.fallbackScriptCache.set(processor.name, fn);
        }
      }

      // Execute with timeout using Promise.race
      const timeoutPromise = new Promise<never>((_, reject) => {
        setTimeout(() => reject(new Error('Processor execution timed out')), this.config.timeoutMs);
      });

      const executionPromise = Promise.resolve().then(() => fn(value, key, processor.args));

      const result = await Promise.race([executionPromise, timeoutPromise]) as { value: V | undefined; result?: R };

      // Validate result format
      if (typeof result !== 'object' || result === null) {
        return {
          success: false,
          error: 'Processor must return { value, result? } object',
        };
      }

      return {
        success: true,
        result: result.result,
        newValue: result.value,
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      return {
        success: false,
        error: message,
      };
    }
  }

  /**
   * Get or create an isolate for a processor.
   */
  private getOrCreateIsolate(name: string): IsolatedVmIsolate {
    if (!ivm) {
      throw new Error('isolated-vm not available');
    }

    let isolate = this.isolateCache.get(name);

    if (!isolate || isolate.isDisposed) {
      // Evict oldest if at capacity
      if (this.isolateCache.size >= this.config.maxCachedIsolates) {
        const oldest = this.isolateCache.keys().next().value;
        if (oldest) {
          const oldIsolate = this.isolateCache.get(oldest);
          if (oldIsolate && !oldIsolate.isDisposed) {
            oldIsolate.dispose();
          }
          this.isolateCache.delete(oldest);
          this.scriptCache.delete(oldest);
        }
      }

      isolate = new ivm.Isolate({
        memoryLimit: this.config.memoryLimitMb,
      });
      this.isolateCache.set(name, isolate);
    }

    return isolate;
  }

  /**
   * Get or compile a script for a processor.
   */
  private async getOrCompileScript(
    name: string,
    code: string,
    isolate: IsolatedVmIsolate,
  ): Promise<IsolatedVmScript> {
    let script = this.scriptCache.get(name);

    if (!script) {
      script = await isolate.compileScript(code);
      this.scriptCache.set(name, script);
    }

    return script;
  }

  /**
   * Clear script cache for a specific processor (e.g., when code changes).
   */
  clearCache(processorName?: string): void {
    if (processorName) {
      const isolate = this.isolateCache.get(processorName);
      if (isolate && !isolate.isDisposed) {
        isolate.dispose();
      }
      this.isolateCache.delete(processorName);
      this.scriptCache.delete(processorName);
      this.fallbackScriptCache.delete(processorName);
    } else {
      // Clear all caches
      for (const isolate of this.isolateCache.values()) {
        if (!isolate.isDisposed) {
          isolate.dispose();
        }
      }
      this.isolateCache.clear();
      this.scriptCache.clear();
      this.fallbackScriptCache.clear();
    }
  }

  /**
   * Check if using secure isolated-vm mode.
   */
  isSecureMode(): boolean {
    return ivm !== null;
  }

  /**
   * Get current cache sizes.
   */
  getCacheStats(): { isolates: number; scripts: number; fallbackScripts: number } {
    return {
      isolates: this.isolateCache.size,
      scripts: this.scriptCache.size,
      fallbackScripts: this.fallbackScriptCache.size,
    };
  }

  /**
   * Dispose of all isolates and clear caches.
   */
  dispose(): void {
    if (this.disposed) return;

    this.disposed = true;
    this.clearCache();
    logger.debug('ProcessorSandbox disposed');
  }
}
