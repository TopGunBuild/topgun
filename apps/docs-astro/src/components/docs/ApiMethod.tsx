import React from 'react';
import type { LucideIcon } from 'lucide-react';

interface ApiMethodProps {
    icon?: LucideIcon;
    iconColor?: string;
    name: string;
    description: string;
    code: string;
}

export const ApiMethod: React.FC<ApiMethodProps> = ({
    icon: Icon,
    iconColor = 'text-neutral-500',
    name,
    description,
    code,
}) => {
    return (
        <div className="mb-8">
            <h3 className="text-xl font-medium text-foreground mb-3 flex items-center gap-2">
                {Icon && <Icon className={`w-5 h-5 ${iconColor}`} />}
                {name}
            </h3>
            <p className="text-neutral-600 dark:text-neutral-300 mb-3">{description}</p>
            <div className="bg-neutral-900 rounded-lg p-4 overflow-x-auto">
                <code
                    className="text-sm font-mono text-neutral-300"
                    dangerouslySetInnerHTML={{ __html: code }}
                />
            </div>
        </div>
    );
};

interface ApiParamProps {
    name: string;
    type: string;
    description: string;
}

export const ApiParam: React.FC<ApiParamProps> = ({ name, type, description }) => {
    return (
        <li className="flex flex-col sm:flex-row gap-2 sm:gap-8">
            <code className="font-mono text-sm text-red-500 w-32 shrink-0">{name}</code>
            <div className="text-sm text-neutral-600 dark:text-neutral-300">
                <span className="font-semibold text-foreground">{type}</span>. {description}
            </div>
        </li>
    );
};

interface ApiConstructorProps {
    signature: string;
    children: React.ReactNode;
}

export const ApiConstructor: React.FC<ApiConstructorProps> = ({ signature, children }) => {
    return (
        <div className="bg-card border border-card-border rounded-xl overflow-hidden">
            <div className="p-4 border-b border-card-border bg-neutral-50 dark:bg-neutral-900/50">
                <code className="text-sm font-mono text-blue-600 dark:text-blue-400">{signature}</code>
            </div>
            <div className="p-6">
                <h4 className="text-sm font-semibold text-neutral-500 uppercase tracking-wider mb-4">
                    Parameters
                </h4>
                <ul className="space-y-4">{children}</ul>
            </div>
        </div>
    );
};
