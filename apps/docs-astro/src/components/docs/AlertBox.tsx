import React from 'react';

type AlertVariant = 'warning' | 'info' | 'success' | 'danger';

interface AlertBoxProps {
    variant?: AlertVariant;
    title: string;
    text: string;
}

const variantStyles: Record<AlertVariant, { container: string; title: string; text: string }> = {
    warning: {
        container: 'bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800',
        title: 'text-amber-800 dark:text-amber-300',
        text: 'text-amber-700 dark:text-amber-400',
    },
    info: {
        container: 'bg-brand-subtle/10 border border-brand-subtle/20',
        title: 'text-brand',
        text: 'text-brand',
    },
    success: {
        container: 'bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800',
        title: 'text-green-800 dark:text-green-300',
        text: 'text-green-700 dark:text-green-400',
    },
    danger: {
        container: 'bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800',
        title: 'text-red-800 dark:text-red-300',
        text: 'text-red-700 dark:text-red-400',
    },
};

export const AlertBox: React.FC<AlertBoxProps> = ({ variant = 'warning', title, text }) => {
    const styles = variantStyles[variant];

    return (
        <div className={`${styles.container} rounded-lg p-4 not-prose`}>
            <h4 className={`font-semibold ${styles.title} mb-2`}>{title}</h4>
            <p className={`text-sm ${styles.text}`}>{text}</p>
        </div>
    );
};
