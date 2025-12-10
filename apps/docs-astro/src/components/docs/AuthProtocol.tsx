import React from 'react';

export const AuthProtocol: React.FC = () => {
    return (
        <div className="bg-neutral-50 dark:bg-neutral-800/50 rounded-xl p-6 font-mono text-sm">
            <div className="space-y-3">
                <div className="flex items-start gap-4">
                    <span className="text-blue-500 font-semibold w-16">Client</span>
                    <span className="text-neutral-400">→</span>
                    <span>Connect to WebSocket</span>
                </div>
                <div className="flex items-start gap-4">
                    <span className="text-green-500 font-semibold w-16">Server</span>
                    <span className="text-neutral-400">→</span>
                    <span>
                        <code className="bg-neutral-200 dark:bg-neutral-700 px-1 rounded">AUTH_REQUIRED</code>
                    </span>
                </div>
                <div className="flex items-start gap-4">
                    <span className="text-blue-500 font-semibold w-16">Client</span>
                    <span className="text-neutral-400">→</span>
                    <span>
                        <code className="bg-neutral-200 dark:bg-neutral-700 px-1 rounded">AUTH</code> + JWT token
                    </span>
                </div>
                <div className="flex items-start gap-4">
                    <span className="text-green-500 font-semibold w-16">Server</span>
                    <span className="text-neutral-400">→</span>
                    <span>Verify JWT, extract principal</span>
                </div>
                <div className="flex items-start gap-4">
                    <span className="text-green-500 font-semibold w-16">Server</span>
                    <span className="text-neutral-400">→</span>
                    <span>
                        <code className="bg-neutral-200 dark:bg-neutral-700 px-1 rounded">AUTH_ACK</code> (success) or
                        close connection (failure)
                    </span>
                </div>
            </div>
        </div>
    );
};
