import React from 'react';
import { Check, X } from 'lucide-react';

type CellValue = string | { icon: 'check' | 'x'; text: string; variant?: 'success' | 'error' };

interface ComparisonRowProps {
    feature: string;
    tg: CellValue;
    es: CellValue;
    fb: CellValue;
    rx: CellValue;
}

const CellContent = ({ value, isTopGun = false }: { value: CellValue; isTopGun?: boolean }) => {
    if (typeof value === 'string') {
        return <>{value}</>;
    }

    const iconClass = value.variant === 'error'
        ? 'text-red-500 dark:text-red-400'
        : isTopGun
            ? ''
            : '';

    return (
        <span className={`flex items-center gap-1 justify-center ${value.variant === 'error' ? 'text-red-500 dark:text-red-400' : ''}`}>
            {value.icon === 'check' && <Check className="w-4 h-4" />}
            {value.icon === 'x' && <X className="w-4 h-4" />}
            {value.text}
        </span>
    );
};

export const ComparisonRow = ({ feature, tg, es, fb, rx }: ComparisonRowProps) => (
    <tr className="border-b border-card-border hover:bg-black/5 dark:hover:bg-white/[0.02] transition-colors">
        <td className="py-4 px-4 text-sm font-medium text-neutral-700 dark:text-neutral-300">{feature}</td>
        <td className="py-4 px-4 text-center bg-blue-500/5 border-x border-blue-500/10">
            <div className="flex justify-center text-blue-600 dark:text-blue-400 font-semibold text-sm">
                <CellContent value={tg} isTopGun />
            </div>
        </td>
        <td className="py-4 px-4 text-center text-neutral-600 dark:text-neutral-300 text-sm">
            <CellContent value={es} />
        </td>
        <td className="py-4 px-4 text-center text-neutral-600 dark:text-neutral-300 text-sm">
            <CellContent value={fb} />
        </td>
        <td className="py-4 px-4 text-center text-neutral-600 dark:text-neutral-300 text-sm">
            <CellContent value={rx} />
        </td>
    </tr>
);
