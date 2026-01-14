import { useEffect, useState, useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { Command } from 'cmdk';
import {
  LayoutDashboard,
  Database,
  Server,
  Terminal,
  Settings,
  Moon,
  Sun,
  ExternalLink,
  Search,
} from 'lucide-react';
import { cn } from '@/lib/utils';

interface CommandItem {
  id: string;
  label: string;
  section: string;
  icon: React.ReactNode;
  shortcut?: string;
  path?: string;
  action?: string;
  url?: string;
}

const COMMANDS: CommandItem[] = [
  // Navigation
  {
    id: 'dashboard',
    label: 'Go to Dashboard',
    section: 'Navigation',
    icon: <LayoutDashboard className="h-4 w-4" />,
    path: '/',
  },
  {
    id: 'explorer',
    label: 'Go to Data Explorer',
    section: 'Navigation',
    icon: <Database className="h-4 w-4" />,
    path: '/explorer',
  },
  {
    id: 'playground',
    label: 'Go to Query Playground',
    section: 'Navigation',
    icon: <Terminal className="h-4 w-4" />,
    path: '/playground',
  },
  {
    id: 'cluster',
    label: 'Go to Cluster Topology',
    section: 'Navigation',
    icon: <Server className="h-4 w-4" />,
    path: '/cluster',
  },
  {
    id: 'settings',
    label: 'Go to Settings',
    section: 'Navigation',
    icon: <Settings className="h-4 w-4" />,
    path: '/settings',
  },
  // Theme
  {
    id: 'theme-dark',
    label: 'Switch to Dark Mode',
    section: 'Theme',
    icon: <Moon className="h-4 w-4" />,
    action: 'dark',
  },
  {
    id: 'theme-light',
    label: 'Switch to Light Mode',
    section: 'Theme',
    icon: <Sun className="h-4 w-4" />,
    action: 'light',
  },
  // Help
  {
    id: 'docs',
    label: 'Open Documentation',
    section: 'Help',
    icon: <ExternalLink className="h-4 w-4" />,
    url: 'https://topgun.build/docs',
  },
];

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const navigate = useNavigate();

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };

    document.addEventListener('keydown', down);
    return () => document.removeEventListener('keydown', down);
  }, []);

  const handleSelect = useCallback(
    (command: CommandItem) => {
      setOpen(false);
      setSearch('');

      if (command.path) {
        navigate(command.path);
      } else if (command.action === 'dark') {
        document.documentElement.classList.add('dark');
        localStorage.setItem('theme', 'dark');
      } else if (command.action === 'light') {
        document.documentElement.classList.remove('dark');
        localStorage.setItem('theme', 'light');
      } else if (command.url) {
        window.open(command.url, '_blank');
      }
    },
    [navigate]
  );

  const sections = ['Navigation', 'Theme', 'Help'];
  const filteredCommands = COMMANDS.filter(
    (c) =>
      !search ||
      c.label.toLowerCase().includes(search.toLowerCase()) ||
      c.section.toLowerCase().includes(search.toLowerCase())
  );

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50">
      {/* Backdrop */}
      <div
        className="fixed inset-0 bg-black/50 backdrop-blur-sm"
        onClick={() => setOpen(false)}
      />

      {/* Dialog */}
      <div className="fixed top-[20%] left-1/2 -translate-x-1/2 w-full max-w-lg">
        <Command className="rounded-lg border shadow-lg bg-popover overflow-hidden">
          <div className="flex items-center border-b px-3">
            <Search className="h-4 w-4 text-muted-foreground mr-2" />
            <Command.Input
              value={search}
              onValueChange={setSearch}
              placeholder="Type a command or search..."
              className="flex h-11 w-full bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground"
            />
            <kbd className="pointer-events-none h-5 select-none items-center gap-1 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground flex">
              ESC
            </kbd>
          </div>

          <Command.List className="max-h-[400px] overflow-y-auto p-2">
            <Command.Empty className="py-6 text-center text-sm text-muted-foreground">
              No results found.
            </Command.Empty>

            {sections.map((section) => {
              const sectionCommands = filteredCommands.filter((c) => c.section === section);
              if (sectionCommands.length === 0) return null;

              return (
                <Command.Group key={section} heading={section}>
                  <p className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
                    {section}
                  </p>
                  {sectionCommands.map((command) => (
                    <Command.Item
                      key={command.id}
                      value={command.label}
                      onSelect={() => handleSelect(command)}
                      className={cn(
                        'flex items-center gap-2 px-2 py-2 rounded-md cursor-pointer text-sm',
                        'aria-selected:bg-accent aria-selected:text-accent-foreground'
                      )}
                    >
                      {command.icon}
                      <span>{command.label}</span>
                      {command.shortcut && (
                        <kbd className="ml-auto pointer-events-none h-5 select-none items-center gap-1 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground hidden sm:flex">
                          {command.shortcut}
                        </kbd>
                      )}
                    </Command.Item>
                  ))}
                </Command.Group>
              );
            })}
          </Command.List>

          <div className="flex items-center justify-between border-t px-3 py-2 text-xs text-muted-foreground">
            <div className="flex items-center gap-2">
              <kbd className="px-1.5 py-0.5 bg-muted rounded">↑↓</kbd>
              <span>Navigate</span>
            </div>
            <div className="flex items-center gap-2">
              <kbd className="px-1.5 py-0.5 bg-muted rounded">↵</kbd>
              <span>Select</span>
            </div>
            <div className="flex items-center gap-2">
              <kbd className="px-1.5 py-0.5 bg-muted rounded">esc</kbd>
              <span>Close</span>
            </div>
          </div>
        </Command>
      </div>
    </div>
  );
}

export default CommandPalette;
