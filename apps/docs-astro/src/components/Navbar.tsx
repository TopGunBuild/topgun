import React, { useState, useEffect } from 'react';
import {
    Github,
    Menu,
    X,
    Sun,
    Moon
} from 'lucide-react';
import { TopGunLogo } from './TopGunLogo';

export const Navbar = ({
    isDark,
    toggleTheme,
}: {
    isDark: boolean;
    toggleTheme: () => void;
}) => {
    const [isOpen, setIsOpen] = useState(false);
    const [scrolled, setScrolled] = useState(false);
    const [pathname, setPathname] = useState('/');

    useEffect(() => {
        setPathname(window.location.pathname);
    }, []);

    // Determine current page for styling
    const isDocs = pathname.startsWith('/docs');
    const isBlog = pathname.startsWith('/blog');
    const isLanding = pathname === '/';

    useEffect(() => {
        const handleScroll = () => {
            setScrolled(window.scrollY > 20);
        };
        window.addEventListener('scroll', handleScroll);
        return () => window.removeEventListener('scroll', handleScroll);
    }, []);

    const handleLinkClick = (hash: string) => {
        if (!isLanding) {
            window.location.href = '/' + hash;
        } else {
            const element = document.querySelector(hash);
            if (element) {
                element.scrollIntoView({ behavior: 'smooth' });
            }
        }
        setIsOpen(false);
    };

    const handleNavigate = (path: string) => {
        window.location.href = path;
        setIsOpen(false);
    }

    return (
        <nav className={`fixed top-0 left-0 right-0 z-50 transition-all duration-300 ${scrolled || !isLanding ? 'border-b border-neutral-200 dark:border-white/5 bg-white/80 dark:bg-black/80 backdrop-blur-md' : 'bg-transparent'}`}>
            <div className={`mx-auto px-4 sm:px-6 lg:px-8 ${isDocs ? 'w-full' : 'max-w-7xl'}`}>
                <div className="flex items-center justify-between h-16">
                    <div className="flex items-center gap-2 cursor-pointer" onClick={() => handleNavigate('/')}>
                        <TopGunLogo />
                        <span className="text-xl font-bold tracking-tight text-foreground">TopGun</span>
                        <span className="ml-2 px-2 py-0.5 text-[10px] font-mono uppercase tracking-wider bg-black/5 dark:bg-white/10 text-neutral-500 dark:text-white/70 rounded-full">Alpha</span>
                    </div>

                    <div className="hidden md:block">
                        <div className="ml-10 flex items-baseline space-x-8">
                            <button onClick={() => handleLinkClick('#features')} className="text-sm font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white transition-colors cursor-pointer">Features</button>
                            <button onClick={() => handleLinkClick('#architecture')} className="text-sm font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white transition-colors cursor-pointer">Architecture</button>
                            <button onClick={() => handleLinkClick('#comparison')} className="text-sm font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white transition-colors cursor-pointer">Comparison</button>
                            <button
                                onClick={() => handleNavigate('/docs/intro')}
                                className={`text-sm font-medium transition-colors cursor-pointer ${isDocs ? 'text-blue-600 dark:text-blue-400' : 'text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white'}`}
                            >
                                Docs
                            </button>
                            <button
                                onClick={() => handleNavigate('/blog')}
                                className={`text-sm font-medium transition-colors cursor-pointer ${isBlog ? 'text-blue-600 dark:text-blue-400' : 'text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white'}`}
                            >
                                Blog
                            </button>
                        </div>
                    </div>

                    <div className="hidden md:flex items-center gap-4">
                        <button
                            onClick={toggleTheme}
                            className="p-2 text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white transition-colors rounded-full hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer"
                            aria-label="Toggle theme"
                        >
                            {isDark ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
                        </button>
                        <a href="https://github.com/TopGunBuild/topgun" target="_blank" rel="noreferrer" className="text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white transition-colors">
                            <Github className="w-5 h-5" />
                        </a>
                        <button
                            onClick={() => handleNavigate('/docs/intro')}
                            className="bg-foreground text-background px-4 py-2 rounded-md text-sm font-semibold hover:opacity-90 transition-opacity cursor-pointer"
                        >
                            Get Started
                        </button>
                    </div>

                    <div className="-mr-2 flex gap-2 md:hidden">
                        <button
                            onClick={toggleTheme}
                            className="p-2 text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white transition-colors cursor-pointer"
                        >
                            {isDark ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
                        </button>
                        <button onClick={() => setIsOpen(!isOpen)} className="text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white cursor-pointer">
                            {isOpen ? <X className="w-6 h-6" /> : <Menu className="w-6 h-6" />}
                        </button>
                    </div>
                </div>
            </div>

            {/* Mobile menu */}
            {isOpen && (
                <div className="md:hidden bg-background/95 backdrop-blur-xl border-b border-neutral-200 dark:border-white/10">
                    <div className="px-2 pt-2 pb-3 space-y-1 sm:px-3">
                        <button onClick={() => handleLinkClick('#features')} className="block w-full text-left px-3 py-2 rounded-md text-base font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer">Features</button>
                        <button onClick={() => handleLinkClick('#architecture')} className="block w-full text-left px-3 py-2 rounded-md text-base font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer">Architecture</button>
                        <button onClick={() => handleLinkClick('#comparison')} className="block w-full text-left px-3 py-2 rounded-md text-base font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer">Comparison</button>
                        <button onClick={() => handleNavigate('/docs/intro')} className="block w-full text-left px-3 py-2 rounded-md text-base font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer">Docs</button>
                        <button onClick={() => handleNavigate('/blog')} className="block w-full text-left px-3 py-2 rounded-md text-base font-medium text-neutral-600 dark:text-neutral-300 hover:text-black dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer">Blog</button>
                        <div className="mt-4 px-3">
                            <button className="w-full bg-foreground text-background px-4 py-2 rounded-md text-sm font-semibold hover:opacity-90 transition-opacity cursor-pointer">
                                Get Started
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </nav>
    );
};
