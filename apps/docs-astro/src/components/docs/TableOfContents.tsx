import React, { useState, useEffect } from 'react';

interface TocItem {
    id: string;
    text: string;
    level: number;
}

export const TableOfContents = () => {
    const [toc, setToc] = useState<TocItem[]>([]);
    const [activeId, setActiveId] = useState<string>('');

    useEffect(() => {
        // Function to extract TOC from current page
        const extractToc = () => {
            // Select h2 and h3, but exclude those with data-toc-exclude attribute
            const headers = Array.from(document.querySelectorAll('main h2:not([data-toc-exclude]), main h3:not([data-toc-exclude])'));
            const items: TocItem[] = headers.map((header) => {
                if (!header.id) {
                    header.id = header.textContent?.toLowerCase().replace(/[^a-z0-9]+/g, '-') || '';
                }
                return {
                    id: header.id,
                    text: header.textContent || '',
                    level: parseInt(header.tagName.substring(1))
                };
            });
            setToc(items);
        };

        // Use requestAnimationFrame + small delay to ensure content is rendered
        let timer: ReturnType<typeof setTimeout>;
        const rafId = requestAnimationFrame(() => {
            timer = setTimeout(extractToc, 100);
        });

        return () => {
            cancelAnimationFrame(rafId);
            clearTimeout(timer);
        };
    }, []);

    useEffect(() => {
        const observer = new IntersectionObserver(
            (entries) => {
                entries.forEach((entry) => {
                    if (entry.isIntersecting) {
                        setActiveId(entry.target.id);
                    }
                });
            },
            { rootMargin: '-100px 0px -66%' }
        );

        const headers = document.querySelectorAll('h2, h3');
        headers.forEach((header) => observer.observe(header));

        return () => observer.disconnect();
    }, [toc]);

    if (toc.length === 0) return null;

    return (
        <div className="py-12 px-6">
            <h4 className="text-sm font-semibold text-foreground mb-4">On this page</h4>
            <div className="space-y-1 text-sm border-l border-card-border">
                <a
                    href="#top"
                    className={`block pl-4 py-1 border-l -ml-px transition-colors ${activeId === '' ? 'text-blue-600 border-blue-500 font-medium' : 'text-neutral-400 hover:text-foreground border-transparent'}`}
                >
                    Top
                </a>
                {toc.map((item) => (
                    <a
                        key={item.id}
                        href={`#${item.id}`}
                        className={`block py-1 border-l -ml-px transition-colors ${item.level === 3 ? 'pl-8' : 'pl-4'
                            } ${activeId === item.id
                                ? 'text-blue-600 border-blue-500 font-medium'
                                : 'text-neutral-400 hover:text-foreground border-transparent'
                            }`}
                    >
                        {item.text}
                    </a>
                ))}
            </div>
        </div>
    );
};
