import { Link, Outlet, useLocation, useNavigate } from 'react-router-dom';
import {
  LayoutDashboard,
  Database,
  Server,
  Terminal,
  Settings,
  LogOut,
  Moon,
  Sun,
  Command,
  WifiOff,
  Wifi,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { TopGunLogo } from '@/components/TopGunLogo';
import { cn } from '@/lib/utils';
import { useEffect, useState } from 'react';
import { useServerStatus } from '@/hooks/useServerStatus';

const navItems = [
  { path: '/', label: 'Dashboard', icon: LayoutDashboard },
  { path: '/explorer', label: 'Data Explorer', icon: Database },
  { path: '/playground', label: 'Query Playground', icon: Terminal },
  { path: '/cluster', label: 'Cluster', icon: Server },
  { path: '/settings', label: 'Settings', icon: Settings },
];

export function Layout() {
  const location = useLocation();
  const navigate = useNavigate();
  const { status, error, loading } = useServerStatus();
  const [isDark, setIsDark] = useState(() => {
    // Check localStorage or system preference
    if (typeof window !== 'undefined') {
      const saved = localStorage.getItem('theme');
      if (saved) return saved === 'dark';
      return window.matchMedia('(prefers-color-scheme: dark)').matches;
    }
    return true; // Default to dark
  });

  useEffect(() => {
    if (isDark) {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
    localStorage.setItem('theme', isDark ? 'dark' : 'light');
  }, [isDark]);

  const handleLogout = () => {
    if (window.confirm('Are you sure you want to logout?')) {
      localStorage.removeItem('topgun_token');
      navigate('/login');
    }
  };

  const toggleTheme = () => {
    setIsDark(!isDark);
  };

  const isActive = (path: string) => {
    if (path === '/') return location.pathname === '/';
    return location.pathname.startsWith(path);
  };

  return (
    <div className="flex h-screen bg-background">
      {/* Sidebar */}
      <div className="w-64 border-r bg-card flex flex-col">
        <div className="p-6 border-b">
          <h1 className="text-xl font-bold flex items-center gap-2">
            <TopGunLogo className="h-6 w-6" />
            TopGun Admin
          </h1>
          {/* Server connection status */}
          <div className="mt-2 flex items-center gap-2 text-xs">
            {loading ? (
              <span className="text-muted-foreground">Checking server...</span>
            ) : error ? (
              <span className="text-destructive flex items-center gap-1">
                <WifiOff className="h-3 w-3" />
                Server disconnected
              </span>
            ) : (
              <span className="text-green-600 dark:text-green-400 flex items-center gap-1">
                <Wifi className="h-3 w-3" />
                Connected
              </span>
            )}
          </div>
        </div>

        <nav className="flex-1 p-4 space-y-1">
          {navItems.map((item) => {
            const Icon = item.icon;
            return (
              <Link
                key={item.path}
                to={item.path}
                className={cn(
                  'flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium transition-colors',
                  isActive(item.path)
                    ? 'bg-primary text-primary-foreground'
                    : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
                )}
              >
                <Icon className="h-5 w-5" />
                {item.label}
              </Link>
            );
          })}
        </nav>

        {/* Keyboard shortcut hint */}
        <div className="p-4 border-t">
          <button
            onClick={() => {
              const event = new KeyboardEvent('keydown', {
                key: 'k',
                metaKey: true,
                bubbles: true,
              });
              document.dispatchEvent(event);
            }}
            className="flex items-center gap-2 w-full px-3 py-2 text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground rounded-md transition-colors"
          >
            <Command className="h-4 w-4" />
            <span>Command Palette</span>
            <kbd className="ml-auto text-xs bg-muted px-1.5 py-0.5 rounded">⌘K</kbd>
          </button>
        </div>

        {/* Bottom actions */}
        <div className="p-4 border-t space-y-1">
          <Button
            variant="ghost"
            size="sm"
            className="w-full justify-start"
            onClick={toggleTheme}
          >
            {isDark ? (
              <>
                <Sun className="h-4 w-4 mr-2" />
                Light Mode
              </>
            ) : (
              <>
                <Moon className="h-4 w-4 mr-2" />
                Dark Mode
              </>
            )}
          </Button>

          <Button
            variant="ghost"
            size="sm"
            className="w-full justify-start text-red-600 dark:text-red-400 hover:text-red-700 dark:hover:text-red-300 hover:bg-red-500/10"
            onClick={handleLogout}
          >
            <LogOut className="h-4 w-4 mr-2" />
            Logout
          </Button>
        </div>
      </div>

      {/* Main Content */}
      <div className="flex-1 overflow-hidden">
        <Outlet />
      </div>
    </div>
  );
}

export default Layout;
