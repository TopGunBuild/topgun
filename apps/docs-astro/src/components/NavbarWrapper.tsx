import React, { useState, useEffect } from 'react';
import { Navbar } from './Navbar';

export const NavbarWrapper = () => {
    const [isDark, setIsDark] = useState(true);

    useEffect(() => {
        // Sync with initial state
        setIsDark(document.documentElement.classList.contains('dark'));

        // Observer for external changes (optional)
        const observer = new MutationObserver(() => {
            setIsDark(document.documentElement.classList.contains('dark'));
        });
        observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
        return () => observer.disconnect();
    }, []);

    const toggleTheme = () => {
        const newVal = !isDark;
        setIsDark(newVal);
        if (newVal) {
            document.documentElement.classList.add('dark');
            localStorage.setItem('theme', 'dark');
        } else {
            document.documentElement.classList.remove('dark');
            localStorage.setItem('theme', 'light');
        }
    };

    return <Navbar isDark={isDark} toggleTheme={toggleTheme} />;
};
