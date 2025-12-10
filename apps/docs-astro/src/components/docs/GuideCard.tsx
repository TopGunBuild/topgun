import React from 'react';
import { ChevronRight } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

interface GuideCardProps {
    href: string;
    icon: LucideIcon;
    title: string;
    description: string;
    comingSoon?: boolean;
}

export const GuideCard: React.FC<GuideCardProps> = ({
    href,
    icon: Icon,
    title,
    description,
    comingSoon = false,
}) => {
    if (comingSoon) {
        return (
            <div className="group block p-6 border border-card-border rounded-xl bg-card opacity-70 hover:opacity-100 transition-opacity">
                <div className="flex items-start gap-4">
                    <div className="p-3 rounded-lg bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300">
                        <Icon className="w-6 h-6" />
                    </div>
                    <div className="flex-1">
                        <div className="flex items-center gap-2 mb-2">
                            <h3 className="text-xl font-semibold text-foreground">{title}</h3>
                            <span className="px-2 py-0.5 text-[10px] font-mono uppercase bg-neutral-200 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 rounded-full">
                                Coming Soon
                            </span>
                        </div>
                        <p className="text-neutral-600 dark:text-neutral-300 mb-4">{description}</p>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <a
            href={href}
            className="group block p-6 border border-card-border rounded-xl bg-card hover:border-blue-500/50 transition-all"
        >
            <div className="flex items-start gap-4">
                <div className="p-3 rounded-lg bg-neutral-100 dark:bg-neutral-800 text-blue-600 dark:text-blue-400 group-hover:text-blue-500 transition-colors">
                    <Icon className="w-6 h-6" />
                </div>
                <div className="flex-1">
                    <div className="flex items-center gap-2 mb-2">
                        <h3 className="text-xl font-semibold text-foreground group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors">
                            {title}
                        </h3>
                    </div>
                    <p className="text-neutral-600 dark:text-neutral-300 mb-4">{description}</p>
                    <span className="text-sm font-medium text-blue-600 dark:text-blue-400 flex items-center gap-1">
                        <span>Read Guide</span>
                        <ChevronRight className="w-4 h-4 group-hover:translate-x-1 transition-transform" />
                    </span>
                </div>
            </div>
        </a>
    );
};
