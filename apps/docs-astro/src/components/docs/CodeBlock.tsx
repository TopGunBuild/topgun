import React, { useState } from 'react';
import { Check, Copy } from 'lucide-react';

interface CodeBlockProps {
    title: string;
    code: string;
    language?: string;
}

export const CodeBlock = ({ title, code }: CodeBlockProps) => {
    const [copied, setCopied] = useState(false);

    const handleCopy = async () => {
        try {
            await navigator.clipboard.writeText(code);
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        } catch {
            console.error('Failed to copy to clipboard');
        }
    };

    return (
        <div className="rounded-lg border border-card-border bg-[#0d0d0d] overflow-hidden my-6">
            <div className="flex items-center justify-between px-4 py-2 border-b border-white/10 bg-white/5">
                <span className="text-xs font-mono text-neutral-400">{title}</span>
                <button onClick={handleCopy} className="text-neutral-500 hover:text-white transition-colors">
                    {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                </button>
            </div>
            <div className="p-4 overflow-x-auto text-sm font-mono">
                <pre className="text-neutral-300 !bg-transparent !p-0 !m-0">
                    <code>{code}</code>
                </pre>
            </div>
        </div>
    );
};
