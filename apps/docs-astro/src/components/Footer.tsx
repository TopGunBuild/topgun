import { Github } from 'lucide-react';

export const Footer = () => {
  return (
    <footer className="border-t border-card-border bg-neutral-100 dark:bg-background py-12 transition-colors">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-8 mb-8">
          <div className="col-span-1 md:col-span-2">
            <h3 className="text-xl font-bold text-foreground mb-4">TopGun</h3>
            <p className="text-neutral-600 dark:text-neutral-300 max-w-sm">
              The hybrid offline-first in-memory data grid for the modern web.
              Built for speed, reliability, and developer experience.
            </p>
          </div>
          <div>
            <h4 className="text-foreground font-semibold mb-4">Product</h4>
            <ul className="space-y-2 text-sm text-neutral-600 dark:text-neutral-300">
              <li><a href="/docs" className="hover:text-black dark:hover:text-white transition-colors">Documentation</a></li>
              <li><a href="/whitepaper" className="hover:text-black dark:hover:text-white transition-colors">Whitepaper</a></li>
              <li><a href="https://github.com/TopGunBuild/topgun" target="_blank" rel="noopener noreferrer" className="hover:text-black dark:hover:text-white transition-colors">GitHub</a></li>
            </ul>
          </div>
        </div>
        <div className="pt-8 border-t border-card-border flex flex-col md:flex-row justify-between items-center gap-4">
          <p className="text-neutral-600 dark:text-neutral-600 text-sm">
            © 2025 TopGun Inc. All rights reserved.
          </p>
          <div className="flex space-x-6">
            <a href="https://github.com/TopGunBuild/topgun" target="_blank" rel="noopener noreferrer" className="text-neutral-500 hover:text-black dark:hover:text-white transition-colors">
              <Github className="w-5 h-5" />
            </a>
          </div>
        </div>
      </div>
    </footer>
  );
};