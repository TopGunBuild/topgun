import { defineConfig, type Options } from "tsup";

export default defineConfig((options: Options) => ({
  entry: ["src/index.ts"],
  clean: true,
  dts: true,
  format: ["cjs", "esm"],
  ...options,
}));
