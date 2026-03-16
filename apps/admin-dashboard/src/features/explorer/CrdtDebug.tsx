/**
 * CRDT Debug panel placeholder component.
 * Displays empty state UI with prepared layout for future CRDT debugging data.
 * Data population will be available when system maps ($sys/*) are implemented.
 */

import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Clock, GitMerge, TreePine, AlertTriangle } from 'lucide-react';

const debugSections = [
  {
    title: 'HLC Clock State',
    description: 'Hybrid Logical Clock timestamps per map',
    icon: Clock,
  },
  {
    title: 'Merge History',
    description: 'Recent CRDT merge operations and conflict resolutions',
    icon: GitMerge,
  },
  {
    title: 'Merkle Tree Summary',
    description: 'Sync state and hash tree comparison across nodes',
    icon: TreePine,
  },
  {
    title: 'Conflict Inspector',
    description: 'Detected conflicts and their resolution outcomes',
    icon: AlertTriangle,
  },
];

export function CrdtDebug() {
  return (
    <div className="space-y-4">
      <div className="text-center py-6">
        <p className="text-muted-foreground">
          CRDT debugging data will be available when system maps are implemented.
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {debugSections.map((section) => {
          const Icon = section.icon;
          return (
            <Card key={section.title} className="opacity-60">
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-2">
                  <Icon className="h-4 w-4" />
                  {section.title}
                </CardTitle>
                <CardDescription className="text-xs">
                  {section.description}
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="h-24 flex items-center justify-center text-xs text-muted-foreground border border-dashed rounded-md">
                  No data available
                </div>
              </CardContent>
            </Card>
          );
        })}
      </div>
    </div>
  );
}

export default CrdtDebug;
