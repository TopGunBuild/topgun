import { Options } from "tsup";

export const tsup: Options = {
    globalName: "topGunSocket",
    splitting: true,
    sourcemap: true,
    clean: true,
    dts: true,
    format: ["cjs", "esm", "iife"],
    minify: true,
    bundle: true,
    skipNodeModulesBundle: true,
    entry: {
        client: "src/client/index.ts",
        server: "src/server/index.ts",
    },
    watch: false,
};
