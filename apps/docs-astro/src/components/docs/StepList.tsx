import React from 'react';

interface StepProps {
    number: number;
    title: string;
    description: string;
}

export const Step: React.FC<StepProps> = ({ number, title, description }) => {
    return (
        <div className="flex gap-4">
            <div className="flex-none w-8 h-8 rounded-full bg-brand-subtle/10 text-brand flex items-center justify-center font-bold">
                {number}
            </div>
            <div>
                <h3 className="font-semibold text-foreground mb-1">{title}</h3>
                <p className="text-neutral-600 dark:text-neutral-300 text-sm">{description}</p>
            </div>
        </div>
    );
};

export const StepList: React.FC<{ children: React.ReactNode }> = ({ children }) => {
    return <div className="space-y-6">{children}</div>;
};
