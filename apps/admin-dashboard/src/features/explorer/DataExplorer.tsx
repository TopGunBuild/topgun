import { useState, useEffect, useCallback, useMemo } from 'react';
import { useMap, useQuery } from '@topgunbuild/react';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { DataTable, type Column } from '@/components/DataTable';
import { JsonEditor } from '@/components/JsonEditor';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { Search, Plus, Pencil, Trash2, RefreshCw, Database } from 'lucide-react';

interface MapInfo {
  name: string;
  entryCount: number;
}

interface MapEntry {
  id: string;
  data: Record<string, unknown>;
  _hlc?: { millis: number; counter: number };
  [key: string]: unknown;
}

export function DataExplorer() {
  const [maps, setMaps] = useState<MapInfo[]>([]);
  const [selectedMap, setSelectedMap] = useState<string | null>(null);
  const [searchFilter, setSearchFilter] = useState('');
  const [mapFilter, setMapFilter] = useState('');
  const [editingRecord, setEditingRecord] = useState<MapEntry | null>(null);
  const [isNewRecord, setIsNewRecord] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  // Get list of maps from system map
  const { data: mapsData } = useQuery('$sys/maps');

  useEffect(() => {
    if (mapsData && Array.isArray(mapsData)) {
      const mapList = mapsData
        .filter((m: { name?: string }) => m?.name && !m.name.startsWith('$sys/'))
        .map((m: { name: string; entryCount?: number }) => ({
          name: m.name,
          entryCount: m.entryCount || 0,
        }));
      setMaps(mapList);
    }
  }, [mapsData]);

  // Get data from selected map using useMap hook
  const mapInstance = useMap(selectedMap || '_empty');

  const entries: MapEntry[] = useMemo(() => {
    if (!selectedMap || !mapInstance) return [];

    // Get all entries from the map using entries() iterator
    const allEntries: MapEntry[] = [];
    for (const [key, value] of mapInstance.entries()) {
      if (typeof key === 'string' && !key.startsWith('_')) {
        const valueObj = value as Record<string, unknown>;
        const matchesFilter = !searchFilter ||
          key.toLowerCase().includes(searchFilter.toLowerCase()) ||
          JSON.stringify(value).toLowerCase().includes(searchFilter.toLowerCase());

        if (matchesFilter) {
          allEntries.push({
            id: key,
            data: valueObj,
            _hlc: valueObj?._hlc as MapEntry['_hlc'],
          });
        }
      }
    }

    return allEntries;
  }, [selectedMap, mapInstance, searchFilter, refreshKey]);

  const filteredMaps = maps.filter((m) =>
    mapFilter ? m.name.toLowerCase().includes(mapFilter.toLowerCase()) : true
  );

  const handleRefresh = useCallback(() => {
    setRefreshKey((k) => k + 1);
  }, []);

  const handleSave = useCallback(async () => {
    if (!editingRecord || !selectedMap || !mapInstance) return;
    try {
      mapInstance.set(editingRecord.id, editingRecord.data);
      setEditingRecord(null);
      setIsNewRecord(false);
      handleRefresh();
    } catch (err) {
      console.error('Failed to save record:', err);
    }
  }, [editingRecord, selectedMap, mapInstance, handleRefresh]);

  const handleDelete = useCallback(
    async (id: string) => {
      if (!selectedMap || !mapInstance || !confirm(`Delete record "${id}"?`)) return;
      try {
        mapInstance.remove(id);
        handleRefresh();
      } catch (err) {
        console.error('Failed to delete record:', err);
      }
    },
    [selectedMap, mapInstance, handleRefresh]
  );

  const handleNewRecord = () => {
    setEditingRecord({ id: '', data: {} });
    setIsNewRecord(true);
  };

  const columns: Column<MapEntry>[] = [
    { key: 'id', label: 'ID', width: 200 },
    {
      key: 'data',
      label: 'Data',
      render: (row) => (
        <pre className="text-xs max-w-md truncate font-mono text-muted-foreground">
          {JSON.stringify(row.data, null, 0)}
        </pre>
      ),
    },
    {
      key: '_hlc',
      label: 'HLC',
      width: 180,
      render: (row) =>
        row._hlc ? (
          <span className="text-xs text-muted-foreground font-mono">
            {row._hlc.millis}:{row._hlc.counter}
          </span>
        ) : (
          <span className="text-muted-foreground">-</span>
        ),
    },
    {
      key: 'actions',
      label: '',
      width: 100,
      render: (row) => (
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              setEditingRecord(row);
              setIsNewRecord(false);
            }}
          >
            <Pencil className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              handleDelete(row.id);
            }}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ),
    },
  ];

  return (
    <div className="grid grid-cols-[280px_1fr] h-full">
      {/* Sidebar: Map list */}
      <div className="border-r p-4 space-y-4 overflow-y-auto">
        <h2 className="font-semibold flex items-center gap-2">
          <Database className="h-5 w-5" />
          Maps
        </h2>
        <div className="relative">
          <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search maps..."
            className="pl-8 text-sm"
            value={mapFilter}
            onChange={(e) => setMapFilter(e.target.value)}
          />
        </div>

        <div className="space-y-1">
          {filteredMaps.length === 0 ? (
            <p className="text-sm text-muted-foreground py-4 text-center">No maps found</p>
          ) : (
            filteredMaps.map((map) => (
              <button
                key={map.name}
                onClick={() => setSelectedMap(map.name)}
                className={cn(
                  'w-full text-left px-3 py-2 rounded-md text-sm transition-colors',
                  selectedMap === map.name
                    ? 'bg-primary text-primary-foreground'
                    : 'hover:bg-muted'
                )}
              >
                <div className="font-medium truncate">{map.name}</div>
                <div className="text-xs opacity-70">{map.entryCount} entries</div>
              </button>
            ))
          )}
        </div>
      </div>

      {/* Main: Data table */}
      <div className="p-4 space-y-4 overflow-y-auto">
        {selectedMap ? (
          <>
            <div className="flex items-center justify-between">
              <h2 className="text-xl font-semibold">{selectedMap}</h2>
              <div className="flex gap-2">
                <div className="relative">
                  <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
                  <Input
                    placeholder="Filter records..."
                    value={searchFilter}
                    onChange={(e) => setSearchFilter(e.target.value)}
                    className="pl-8 w-64 text-sm"
                  />
                </div>
                <Button variant="outline" size="icon" onClick={handleRefresh}>
                  <RefreshCw className="h-4 w-4" />
                </Button>
                <Button variant="outline" onClick={handleNewRecord}>
                  <Plus className="mr-2 h-4 w-4" />
                  New Record
                </Button>
              </div>
            </div>

            <DataTable
              data={entries}
              columns={columns}
              onRowClick={(row) => {
                setEditingRecord(row);
                setIsNewRecord(false);
              }}
              emptyMessage="No records in this map"
            />
          </>
        ) : (
          <div className="flex flex-col items-center justify-center h-64 text-muted-foreground">
            <Database className="h-12 w-12 mb-4 opacity-50" />
            <p>Select a map to view data</p>
          </div>
        )}
      </div>

      {/* Edit Dialog */}
      <Dialog open={!!editingRecord} onOpenChange={() => setEditingRecord(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{isNewRecord ? 'New Record' : `Edit Record: ${editingRecord?.id}`}</DialogTitle>
          </DialogHeader>

          {isNewRecord && (
            <div className="space-y-2">
              <label className="text-sm font-medium">Record ID</label>
              <Input
                value={editingRecord?.id || ''}
                onChange={(e) =>
                  setEditingRecord((prev) => (prev ? { ...prev, id: e.target.value } : null))
                }
                placeholder="Enter unique ID"
              />
            </div>
          )}

          <JsonEditor
            value={editingRecord?.data}
            onChange={(data) =>
              setEditingRecord((prev) =>
                prev ? { ...prev, data: data as Record<string, unknown> } : null
              )
            }
            className="min-h-[300px]"
          />

          <DialogFooter>
            <Button variant="outline" onClick={() => setEditingRecord(null)}>
              Cancel
            </Button>
            <Button onClick={handleSave}>Save</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export default DataExplorer;
