import { useState, useMemo } from 'react';
import { useAppStore } from '../../store';
import { Database, Folder, Table, RefreshCw, Search, ChevronRight, ChevronDown, Binary } from 'lucide-react';
import type { TableInfo, RoutineInfo } from '../../types';

interface TreeNode {
    id: string;
    label: string;
    type: 'schema' | 'table' | 'view' | 'routine' | 'column';
    icon?: React.ReactNode;
    children?: TreeNode[];
    data?: any;
}

export function DatabaseExplorer() {
    const schemaInfo = useAppStore(s => s.schemaInfo);
    const schemaLoading = useAppStore(s => s.schemaLoading);
    const refreshSchema = useAppStore(s => s.refreshSchema);
    const createTab = useAppStore(s => s.createTab);
    const activeSpaceId = useAppStore(s => s.activeSpaceId);
    const spaces = useAppStore(s => s.spaces);

    // State Persistence
    const expandedNodes = useAppStore(s => s.expandedNodes);
    const toggleNodeExpansion = useAppStore(s => s.toggleNodeExpansion);

    const [searchQuery, setSearchQuery] = useState('');

    // Get active space color for theming
    const activeSpace = spaces.find(s => s.id === activeSpaceId);
    const spaceColor = activeSpace?.color || '#6366f1';

    // Convert SchemaInfo to Tree Structure
    const treeData = useMemo(() => {
        if (!schemaInfo) return [];

        const rootNodes: TreeNode[] = [];

        // Group by Schema
        const tablesBySchema = new Map<string, TableInfo[]>();
        const viewsBySchema = new Map<string, TableInfo[]>();
        const routinesBySchema = new Map<string, RoutineInfo[]>();

        // Process Tables & Views
        schemaInfo.tables.forEach(table => {
            if (table.table_type === 'VIEW') {
                const list = viewsBySchema.get(table.schema_name) || [];
                list.push(table);
                viewsBySchema.set(table.schema_name, list);
            } else {
                const list = tablesBySchema.get(table.schema_name) || [];
                list.push(table);
                tablesBySchema.set(table.schema_name, list);
            }
        });

        // Process Routines
        schemaInfo.routines.forEach(routine => {
            const list = routinesBySchema.get(routine.schema_name) || [];
            list.push(routine);
            routinesBySchema.set(routine.schema_name, list);
        });

        // Build Nodes for each Schema
        schemaInfo.schemas.forEach(schemaName => {
            if (schemaName === 'Refresh') return; // Filter out "Refresh" artifact from tree

            const children: TreeNode[] = [];

            // Tables
            const tables = tablesBySchema.get(schemaName);
            if (tables && tables.length > 0) {
                children.push({
                    id: `schema-${schemaName}-tables`,
                    label: 'Tables',
                    type: 'folder',
                    icon: <Folder className="w-3 h-3 text-blue-400 flex-shrink-0" />,
                    children: tables.map(t => ({
                        id: `table-${schemaName}-${t.table_name}`,
                        label: t.table_name,
                        type: 'table',
                        icon: <Table className="w-3 h-3 text-blue-300 flex-shrink-0" />,
                        data: t
                    }))
                } as any);
            }

            // Views
            const views = viewsBySchema.get(schemaName);
            if (views && views.length > 0) {
                children.push({
                    id: `schema-${schemaName}-views`,
                    label: 'Views',
                    type: 'folder',
                    icon: <Folder className="w-3 h-3 text-purple-400 flex-shrink-0" />,
                    children: views.map(v => ({
                        id: `view-${schemaName}-${v.table_name}`,
                        label: v.table_name,
                        type: 'view',
                        icon: <Table className="w-3 h-3 text-purple-300 flex-shrink-0" />,
                        data: v
                    }))
                } as any);
            }

            // Routines
            const routines = routinesBySchema.get(schemaName);
            if (routines && routines.length > 0) {
                children.push({
                    id: `schema-${schemaName}-routines`,
                    label: 'Procedures',
                    type: 'folder',
                    icon: <Folder className="w-3 h-3 text-orange-400 flex-shrink-0" />,
                    children: routines.map(r => ({
                        id: `routine-${schemaName}-${r.routine_name}`,
                        label: r.routine_name,
                        type: 'routine',
                        icon: <Binary className="w-3 h-3 text-orange-300 flex-shrink-0" />,
                        data: r
                    }))
                } as any);
            }

            if (children.length > 0) {
                rootNodes.push({
                    id: `schema-${schemaName}`,
                    label: schemaName,
                    type: 'schema',
                    icon: <Folder className="w-3 h-3 text-[var(--text-secondary)] flex-shrink-0" />,
                    children
                });
            }
        });

        return rootNodes;
    }, [schemaInfo]);

    // Filter Tree
    const filteredTree = useMemo(() => {
        if (!searchQuery.trim()) return treeData;

        const lowerQuery = searchQuery.toLowerCase();

        // Recursive filter function
        const filterNode = (node: TreeNode): TreeNode | null => {
            const matchesRequest = node.label.toLowerCase().includes(lowerQuery);

            let filteredChildren: TreeNode[] | undefined;
            if (node.children) {
                filteredChildren = node.children
                    .map(filterNode)
                    .filter((child): child is TreeNode => child !== null);
            }

            if (matchesRequest || (filteredChildren && filteredChildren.length > 0)) {
                return {
                    ...node,
                    children: filteredChildren
                };
            }

            return null;
        };

        return treeData.map(filterNode).filter((node): node is TreeNode => node !== null);
    }, [treeData, searchQuery]);

    const handleDoubleClick = async (node: TreeNode) => {
        if (node.type === 'table' || node.type === 'view') {
            if (!activeSpaceId) return;
            const tableInfo = node.data as TableInfo;
            const query = `SELECT TOP 100 * FROM [${tableInfo.schema_name}].[${tableInfo.table_name}]`;
            await createTab(tableInfo.table_name, 'query', query, schemaInfo?.database_name);
        }
    };

    // Recursive Tree Render
    const renderTree = (nodes: TreeNode[]) => {
        return nodes.map(node => {
            const isExpanded = expandedNodes.has(node.id) || !!searchQuery; // Always expand on search
            const hasChildren = node.children && node.children.length > 0;

            return (
                <div key={node.id} className="select-none">
                    <div
                        className={`
                        flex items-center gap-1.5 px-2 py-0.5 text-xs rounded-sm cursor-pointer 
                        hover:bg-[var(--bg-hover)] transition-all duration-200 group relative
                        ${hasChildren ? '' : 'pl-6'} 
                    `}
                        style={{ paddingLeft: hasChildren ? undefined : '1.5rem' }}
                        onClick={() => hasChildren && toggleNodeExpansion(node.id)}
                        onDoubleClick={() => handleDoubleClick(node)}
                    >
                        {/* Hover Effect using space color - thinner and stronger */}
                        <div
                            className="absolute left-0 top-1/2 -translate-y-1/2 h-3 w-0.5 rounded-full opacity-0 group-hover:opacity-100 transition-opacity"
                            style={{ backgroundColor: spaceColor }}
                        />

                        {hasChildren && (
                            <div className="text-[var(--text-muted)] group-hover:text-[var(--text-secondary)] transition-colors flex-shrink-0">
                                {isExpanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                            </div>
                        )}
                        <div className={`flex-shrink-0 transition-transform duration-200 ${isExpanded && hasChildren ? 'scale-110' : ''}`}>
                            {/* Clone element to override size if needed, or rely on class replacement if possible. 
                                 However, the icons are passed as Nodes. We should probably update the generation logic for icons too,
                                 but for now let's hope the flex-shrink-0 and surrounding text-xs helps, or CSS override. 
                                 Actually, we defined the icons with specific classes in renderTree props (in useMemo).
                                 We need to update the useMemo to use smaller icons.
                             */}
                            {node.icon}
                        </div>
                        <span className="truncate text-[var(--text-primary)] group-hover:translate-x-0.5 transition-transform duration-200">{node.label}</span>
                    </div>

                    {hasChildren && isExpanded && (
                        <div className="pl-3 border-l border-[var(--border-subtle)] ml-2.5 mt-0.5">
                            {renderTree(node.children!)}
                        </div>
                    )}
                </div>
            );
        });
    };

    if (schemaLoading && !schemaInfo) {
        return (
            <div className="flex items-center justify-center h-full text-[var(--text-muted)] gap-2">
                <RefreshCw className="w-4 h-4 animate-spin flex-shrink-0" />
                <span>Loading schema...</span>
            </div>
        );
    }

    if (!schemaInfo) {
        return (
            <div className="flex flex-col items-center justify-center h-full text-[var(--text-muted)] p-4 text-center">
                <Database className="w-8 h-8 mb-2 opacity-50 flex-shrink-0" />
                <p>No schema loaded</p>
                <p className="text-xs mt-1 opacity-70">Connect to a database to view its objects.</p>
            </div>
        );
    }

    return (
        <div className="flex flex-col h-full">
            {/* Header */}
            <div className="px-2 py-1.5 flex items-center justify-between border-b border-[var(--border-subtle)] shrink-0">
                <div className="flex items-center gap-1.5 overflow-hidden">
                    <Database className="w-3 h-3 flex-shrink-0" style={{ color: spaceColor }} />
                    <span className="text-xs font-semibold truncate" title={schemaInfo.database_name}>
                        {schemaInfo.database_name}
                    </span>
                </div>
                <button
                    onClick={() => refreshSchema()}
                    className="p-0.5 rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors flex-shrink-0"
                    title="Refresh Schema"
                >
                    <RefreshCw className={`w-3 h-3 ${schemaLoading ? 'animate-spin' : ''}`} />
                </button>
            </div>

            {/* Search */}
            <div className="px-2 py-1.5 shrink-0">
                <div className="relative">
                    <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-[var(--text-muted)] flex-shrink-0" />
                    <input
                        type="text"
                        placeholder="Filter objects..."
                        value={searchQuery}
                        onChange={e => setSearchQuery(e.target.value)}
                        className="w-full h-7 bg-[var(--bg-primary)]/40 hover:bg-[var(--bg-primary)]/60 focus:bg-[var(--bg-primary)]/60 border border-transparent hover:border-[var(--border-subtle)] focus:border-[var(--accent-color)] rounded-md pl-8 pr-2 py-0.5 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] shadow-sm transition-all duration-200 outline-none"
                        style={{
                            '--accent-color': spaceColor
                        } as React.CSSProperties}
                    />
                </div>
            </div>

            {/* Tree Content - Overflow handled here */}
            <div className="flex-1 overflow-y-auto px-1 pb-2 min-h-0">
                {filteredTree.length === 0 ? (
                    <div className="text-center py-4 text-xs text-[var(--text-muted)]">
                        No objects found.
                    </div>
                ) : (
                    renderTree(filteredTree)
                )}
            </div>
        </div>
    );
}
