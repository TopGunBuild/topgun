import { EventEmitter } from '../event-emitter';

describe('EventEmitter', () => {
  interface TestEvents {
    'test:event': { data: string };
    'test:number': number;
    'test:void': void;
  }

  let emitter: EventEmitter<TestEvents>;

  beforeEach(() => {
    emitter = new EventEmitter<TestEvents>();
  });

  afterEach(() => {
    emitter.removeAllListeners();
  });

  describe('on/emit', () => {
    it('should emit and receive events', () => {
      const handler = jest.fn();
      emitter.on('test:event', handler);

      const testData = { data: 'test' };
      emitter.emit('test:event', testData);

      expect(handler).toHaveBeenCalledWith(testData);
    });

    it('should handle multiple subscribers', () => {
      const handler1 = jest.fn();
      const handler2 = jest.fn();

      emitter.on('test:event', handler1);
      emitter.on('test:event', handler2);

      const testData = { data: 'test' };
      emitter.emit('test:event', testData);

      expect(handler1).toHaveBeenCalledWith(testData);
      expect(handler2).toHaveBeenCalledWith(testData);
    });

    it('should return unsubscribe function', () => {
      const handler = jest.fn();
      const unsubscribe = emitter.on('test:event', handler);

      unsubscribe();
      emitter.emit('test:event', { data: 'test' });

      expect(handler).not.toHaveBeenCalled();
    });
  });

  describe('once', () => {
    it('should only trigger once', () => {
      const handler = jest.fn();
      emitter.once('test:event', handler);

      const testData = { data: 'test' };
      emitter.emit('test:event', testData);
      emitter.emit('test:event', testData);

      expect(handler).toHaveBeenCalledTimes(1);
      expect(handler).toHaveBeenCalledWith(testData);
    });

    it('should return unsubscribe function that works before emission', () => {
      const handler = jest.fn();
      const unsubscribe = emitter.once('test:event', handler);

      unsubscribe();
      emitter.emit('test:event', { data: 'test' });

      expect(handler).not.toHaveBeenCalled();
    });
  });

  describe('waitFor', () => {
    it('should resolve with emitted data', async () => {
      const testData = { data: 'test' };
      const promise = emitter.waitFor('test:event');
      
      emitter.emit('test:event', testData);
      const result = await promise;

      expect(result).toEqual(testData);
    });

    it('should resolve only once', async () => {
      const testData1 = { data: 'test1' };
      const testData2 = { data: 'test2' };
      
      const promise = emitter.waitFor('test:event');
      
      emitter.emit('test:event', testData1);
      emitter.emit('test:event', testData2);
      
      const result = await promise;
      expect(result).toEqual(testData1);
    });
  });

  describe('off', () => {
    it('should remove specific handler', () => {
      const handler1 = jest.fn();
      const handler2 = jest.fn();

      emitter.on('test:event', handler1);
      emitter.on('test:event', handler2);

      emitter.off('test:event', handler1);
      emitter.emit('test:event', { data: 'test' });

      expect(handler1).not.toHaveBeenCalled();
      expect(handler2).toHaveBeenCalled();
    });
  });

  describe('removeAllListeners', () => {
    it('should remove all event handlers', () => {
      const handler1 = jest.fn();
      const handler2 = jest.fn();

      emitter.on('test:event', handler1);
      emitter.on('test:number', handler2);

      emitter.removeAllListeners();

      emitter.emit('test:event', { data: 'test' });
      emitter.emit('test:number', 42);

      expect(handler1).not.toHaveBeenCalled();
      expect(handler2).not.toHaveBeenCalled();
    });
  });

  describe('listenerCount', () => {
    it('should return correct number of listeners', () => {
      expect(emitter.listenerCount('test:event')).toBe(0);

      const handler1 = jest.fn();
      const handler2 = jest.fn();

      emitter.on('test:event', handler1);
      expect(emitter.listenerCount('test:event')).toBe(1);

      emitter.on('test:event', handler2);
      expect(emitter.listenerCount('test:event')).toBe(2);

      emitter.off('test:event', handler1);
      expect(emitter.listenerCount('test:event')).toBe(1);
    });
  });

  describe('error handling', () => {
    it('should catch and continue on handler error', () => {
      const errorHandler = jest.fn(() => {
        throw new Error('Test error');
      });
      const normalHandler = jest.fn();
      const consoleSpy = jest.spyOn(console, 'error').mockImplementation();

      emitter.on('test:event', errorHandler);
      emitter.on('test:event', normalHandler);

      emitter.emit('test:event', { data: 'test' });

      expect(normalHandler).toHaveBeenCalled();
      expect(consoleSpy).toHaveBeenCalled();

      consoleSpy.mockRestore();
    });
  });

  describe('type safety', () => {
    it('should handle void events', () => {
      const handler = jest.fn();
      emitter.on('test:void', handler);
      
      // @ts-expect-error - Testing type safety
      emitter.emit('test:void', { data: 'test' });
      
      emitter.emit('test:void', undefined);
      expect(handler).toHaveBeenCalledWith(undefined);
    });

    it('should handle primitive type events', () => {
      const handler = jest.fn();
      emitter.on('test:number', handler);
      
      emitter.emit('test:number', 42);
      expect(handler).toHaveBeenCalledWith(42);
    });
  });
}); 