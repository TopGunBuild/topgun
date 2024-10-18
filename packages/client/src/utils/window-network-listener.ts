import { windowOrGlobal } from "@topgunbuild/utils";
import { NetworkListenerAdapter } from "../types";

export class WindowNetworkListener implements NetworkListenerAdapter {

    /**
     * Get the current network status
     * @returns True if the network is online, false otherwise
     */
    public isOnline(): boolean {
        return windowOrGlobal?.navigator?.onLine ?? false;
    }

    /**
     * Listen for changes to the network status
     * @param f - The function to call when the network status changes
     * @returns A function to stop listening for changes
     */
    public listen(f: (isOnline: boolean) => void): () => void {
        const onOnline = () => {
            f(true);
        };
        const onOffline = () => {
            f(false);
        };

        windowOrGlobal?.addEventListener("online", onOnline);
        windowOrGlobal?.addEventListener("offline", onOffline);

        return () => {
            windowOrGlobal?.removeEventListener("online", onOnline);
            windowOrGlobal?.removeEventListener("offline", onOffline);
        };
    }
}
