import { useState } from 'react';
import { useQuery } from '@topgunbuild/react';
import { Database, Search } from 'lucide-react';
import type { MapInfo, MapEntry } from '../types/system';

export function Maps() {
    const [selectedMap, setSelectedMap] = useState<string | null>(null);
    const [searchKey, setSearchKey] = useState('');

    const { data: mapsData, loading: mapsLoading } = useQuery('$sys/maps', {});

    // Filter out system maps ($sys/*)
    const maps = (mapsData as MapInfo[]).filter((m) => !m.name?.startsWith('$sys/'));

    return (
        <div className="flex h-full">
            {/* Sidebar - map list */}
            <div className="w-64 border-r bg-gray-50 overflow-y-auto">
                <div className="p-4 border-b">
                    <h2 className="font-semibold text-gray-700">Maps</h2>
                </div>
                {mapsLoading ? (
                    <div className="p-4 text-center">
                        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-500 mx-auto"></div>
                        <p className="mt-2 text-sm text-gray-500">Loading...</p>
                    </div>
                ) : maps.length === 0 ? (
                    <div className="p-4 text-center text-gray-500">No maps found</div>
                ) : (
                    <ul>
                        {maps.map((map) => (
                            <li
                                key={map._key || map.name}
                                onClick={() => setSelectedMap(map.name)}
                                className={`px-4 py-3 cursor-pointer hover:bg-gray-100
                                    ${selectedMap === map.name ? 'bg-blue-100 border-r-2 border-blue-500' : ''}`}
                            >
                                <div className="flex items-center gap-2">
                                    <Database size={16} className="text-gray-400" />
                                    <span className="truncate">{map.name}</span>
                                </div>
                                {map.count !== undefined && (
                                    <div className="text-xs text-gray-400 ml-6">{map.count} entries</div>
                                )}
                            </li>
                        ))}
                    </ul>
                )}
            </div>

            {/* Main content - map contents */}
            <div className="flex-1 overflow-auto">
                {selectedMap ? (
                    <MapViewer mapName={selectedMap} searchKey={searchKey} onSearchChange={setSearchKey} />
                ) : (
                    <div className="flex items-center justify-center h-full text-gray-500">
                        <div className="text-center">
                            <Database size={48} className="mx-auto text-gray-300" />
                            <p className="mt-4">Select a map to view its contents</p>
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
}

// Component for displaying map contents
function MapViewer({ mapName, searchKey, onSearchChange }: {
    mapName: string;
    searchKey: string;
    onSearchChange: (value: string) => void;
}) {
    const { data, loading } = useQuery(mapName, {});

    const entries = (data as MapEntry[]).filter((item) =>
        (item._key || '').toLowerCase().includes(searchKey.toLowerCase())
    );

    if (loading) {
        return (
            <div className="flex items-center justify-center h-full">
                <div className="text-center">
                    <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500 mx-auto"></div>
                    <p className="mt-4 text-gray-600">Loading map data...</p>
                </div>
            </div>
        );
    }

    return (
        <div className="p-4">
            <div className="flex items-center justify-between mb-4">
                <h2 className="text-xl font-semibold">{mapName}</h2>
                <div className="relative">
                    <Search size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400" />
                    <input
                        type="text"
                        placeholder="Filter by key..."
                        value={searchKey}
                        onChange={(e) => onSearchChange(e.target.value)}
                        className="pl-9 pr-4 py-2 border rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                </div>
            </div>

            {entries.length === 0 ? (
                <div className="text-center text-gray-500 py-8">
                    {searchKey ? 'No matching entries' : 'This map is empty'}
                </div>
            ) : (
                <div className="bg-white rounded-lg shadow overflow-hidden">
                    <table className="min-w-full">
                        <thead className="bg-gray-50">
                            <tr>
                                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Key</th>
                                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Value</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y divide-gray-200">
                            {entries.map((item) => {
                                // Extract value without _key metadata
                                const { _key, ...value } = item;
                                return (
                                    <tr key={_key} className="hover:bg-gray-50">
                                        <td className="px-4 py-3 font-mono text-sm text-gray-900 whitespace-nowrap align-top">
                                            {_key}
                                        </td>
                                        <td className="px-4 py-3">
                                            <pre className="text-xs text-gray-600 max-w-xl overflow-auto bg-gray-50 p-2 rounded">
                                                {JSON.stringify(value, null, 2)}
                                            </pre>
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
            )}

            <div className="mt-4 text-sm text-gray-500">
                {entries.length} {entries.length === 1 ? 'entry' : 'entries'}
            </div>
        </div>
    );
}
