import { useState, useMemo } from 'react';
import { useAppStore } from '../../store';
import { Database, Folder, Table, RefreshCw, Search, ChevronRight, ChevronDown, Binary, Eye, X } from 'lucide-react';
import type { TableInfo, RoutineInfo } from '../../types';
import { driverOf } from '../../types';
import { mongoFindQuery } from '../../utils/mongo/shell';

interface TreeNode {
    id: string;
    label: string;
    type: 'schema' | 'folder' | 'table' | 'view' | 'routine' | 'column';
    icon?: React.ReactNode;
    children?: TreeNode[];
    data?: any;
}

export function DatabaseExplorer() {
    const schemaInfo = useAppStore(s => s.schemaInfo);
    const schemaLoading = useAppStore(s => s.schemaLoading);
    const refreshSchema = useAppStore(s => s.refreshSchema);
    const createTab = useAppStore(s => s.createTab);
    const openTablePeek = useAppStore(s => s.openTablePeek);
    const openObjectPeek = useAppStore(s => s.openObjectPeek);
    const openTableSourcePeek = useAppStore(s => s.openTableSourcePeek);
    const activeSpaceId = useAppStore(s => s.activeSpaceId);
    const spaces = useAppStore(s => s.spaces);

    // State Persistence
    const expandedNodes = useAppStore(s => s.expandedNodes);
    const toggleNodeExpansion = useAppStore(s => s.toggleNodeExpansion);

    const [searchQuery, setSearchQuery] = useState('');

    // Get active space color for theming
    const activeSpace = spaces.find(s => s.id === activeSpaceId);
    const spaceColor = activeSpace?.color || '#0078d4';
    const isMongo = driverOf(activeSpace?.connection_driver) === 'mongodb';
    const isPostgres = driverOf(activeSpace?.connection_driver) === 'postgres';

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
                    label: isMongo ? 'Collections' : 'Tables',
                    type: 'folder',
                    icon: <Folder className="w-3.5 h-3.5 text-blue-400 flex-shrink-0" />,
                    children: tables.map(t => ({
                        id: `table-${schemaName}-${t.table_name}`,
                        label: t.table_name,
                        type: 'table',
                        icon: <Table className="w-3.5 h-3.5 text-blue-300 flex-shrink-0" />,
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
                    icon: <Folder className="w-3.5 h-3.5 text-purple-400 flex-shrink-0" />,
                    children: views.map(v => ({
                        id: `view-${schemaName}-${v.table_name}`,
                        label: v.table_name,
                        type: 'view',
                        icon: <Eye className="w-3.5 h-3.5 text-purple-300 flex-shrink-0" />,
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
                    icon: <Folder className="w-3.5 h-3.5 text-orange-400 flex-shrink-0" />,
                    children: routines.map(r => ({
                        id: `routine-${schemaName}-${r.routine_name}`,
                        label: r.routine_name,
                        type: 'routine',
                        icon: <Binary className="w-3.5 h-3.5 text-orange-300 flex-shrink-0" />,
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

        // A single schema (typically dbo) is just indentation noise - show its groups directly.
        if (rootNodes.length === 1 && rootNodes[0].children) return rootNodes[0].children;
        return rootNodes;
    }, [schemaInfo, isMongo]);

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
            const query = isMongo
                ? mongoFindQuery(tableInfo.table_name, 100)
                : isPostgres
                  ? `SELECT * FROM "${tableInfo.schema_name}"."${tableInfo.table_name}" LIMIT 100`
                  : `SELECT TOP 100 * FROM [${tableInfo.schema_name}].[${tableInfo.table_name}]`;
            await createTab(tableInfo.table_name, 'query', query, schemaInfo?.database_name);
        }
    };

    // Arc-style "peek": Shift+click previews table/view data, Alt+click previews
    // the source of a view/procedure. Returns true if a peek was triggered.
    const handleModifierClick = (e: React.MouseEvent, node: TreeNode): boolean => {
        // Shift+click a table or view -> TOP 100 data preview
        if (e.shiftKey && (node.type === 'table' || node.type === 'view')) {
            openTablePeek(node.data as TableInfo);
            return true;
        }
        // Definition peeks read SQL Server catalogs
        if (isMongo || isPostgres) return false;
        // Alt+click a table -> CREATE TABLE definition preview
        if (e.altKey && node.type === 'table') {
            openTableSourcePeek(node.data as TableInfo);
            return true;
        }
        // Alt+click a view or procedure -> source definition preview
        if (e.altKey && node.type === 'view') {
            const v = node.data as TableInfo;
            openObjectPeek('view', v.schema_name, v.table_name);
            return true;
        }
        if (e.altKey && node.type === 'routine') {
            const r = node.data as RoutineInfo;
            openObjectPeek('routine', r.schema_name, r.routine_name);
            return true;
        }
        return false;
    };

    const handleNodeClick = (e: React.MouseEvent, node: TreeNode, hasChildren: boolean) => {
        if (handleModifierClick(e, node)) return;
        if (hasChildren) toggleNodeExpansion(node.id);
    };

    // Highlight the filter match inside a label
    const renderLabel = (label: string) => {
        const q = searchQuery.trim();
        if (!q) return label;
        const idx = label.toLowerCase().indexOf(q.toLowerCase());
        if (idx < 0) return label;
        return (
            <>
                {label.slice(0, idx)}
                <mark className="bg-transparent font-semibold" style={{ color: spaceColor }}>
                    {label.slice(idx, idx + q.length)}
                </mark>
                {label.slice(idx + q.length)}
            </>
        );
    };

    const matchCount = useMemo(() => {
        if (!searchQuery.trim()) return 0;
        const count = (nodes: TreeNode[]): number =>
            nodes.reduce((n, node) => n + (node.children ? count(node.children) : 1), 0);
        return count(filteredTree);
    }, [filteredTree, searchQuery]);

    // Recursive Tree Render
    const renderTree = (nodes: TreeNode[]) => {
        return nodes.map(node => {
            const isExpanded = expandedNodes.has(node.id) || !!searchQuery; // Always expand on search
            const hasChildren = node.children && node.children.length > 0;

            return (
                <div key={node.id} className="select-none">
                    <div
                        className={`
                        flex items-center gap-1.5 pl-1.5 pr-1 py-1 text-[13px] rounded-md cursor-pointer
                        hover:bg-[var(--bg-hover)] transition-colors duration-100 group relative
                        ${node.type === 'folder' ? 'sticky top-0 z-[1] bg-[var(--bg-secondary)] hover:bg-[var(--bg-hover)] font-medium' : ''}
                    `}
                        style={{ paddingLeft: hasChildren ? undefined : '1.375rem' }}
                        onClick={(e) => handleNodeClick(e, node, !!hasChildren)}
                        onDoubleClick={() => handleDoubleClick(node)}
                        title={
                            isMongo && (node.type === 'table' || node.type === 'view')
                                ? 'Shift+click to peek documents · double-click to open'
                                : node.type === 'table'
                                ? 'Shift+click to peek data · Alt+click to peek definition · double-click to open'
                                : node.type === 'view'
                                    ? 'Shift+click to peek data · Alt+click to peek source'
                                    : node.type === 'routine'
                                        ? 'Alt+click to peek source'
                                        : undefined
                        }
                    >
                        {/* Hover Effect using space color - thinner and stronger */}
                        <div
                            className="absolute left-0 top-1/2 -translate-y-1/2 h-4 w-[3px] rounded-full opacity-0 group-hover:opacity-100 transition-opacity"
                            style={{ backgroundColor: spaceColor }}
                        />

                        {hasChildren && (
                            <div className="text-[var(--text-muted)] group-hover:text-[var(--text-secondary)] transition-colors flex-shrink-0">
                                {isExpanded ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronRight className="w-3.5 h-3.5" />}
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
                        <div className="overflow-hidden min-w-0 animate-marquee-hover">
                          <span
                            className="marquee-content truncate text-[var(--text-primary)] group-hover:translate-x-0.5 transition-transform duration-200"
                            title={node.label}
                          >
                            {renderLabel(node.label)}
                          </span>
                        </div>
                        {node.type === 'folder' && node.children && (
                            <span className="ml-auto text-[11px] text-[var(--text-muted)] tabular-nums shrink-0">{node.children.length}</span>
                        )}
                    </div>

                    {hasChildren && isExpanded && (
                        <div className="pl-2 border-l border-[var(--border-subtle)] ml-3">
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
            {/* Filter + refresh (database name is already shown in the selector above) */}
            <div className="pl-2 pr-1 py-1.5 shrink-0 flex items-center gap-1">
                <div className="relative flex-1 min-w-0">
                    <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-[var(--text-muted)] flex-shrink-0" />
                    <input
                        type="text"
                        placeholder="Filter objects"
                        value={searchQuery}
                        onChange={e => setSearchQuery(e.target.value)}
                        onKeyDown={e => { if (e.key === 'Escape') setSearchQuery(''); }}
                        className="w-full h-8 bg-[var(--bg-tertiary)] rounded-md pl-8 pr-14 text-[13px] text-[var(--text-primary)] placeholder-[var(--text-muted)] outline-none"
                        style={{ '--accent-color': spaceColor } as React.CSSProperties}
                    />
                    {searchQuery && (
                        <div className="absolute right-1.5 top-1/2 -translate-y-1/2 flex items-center gap-1">
                            <span className="text-[11px] text-[var(--text-muted)] tabular-nums">{matchCount}</span>
                            <button
                                onClick={() => setSearchQuery('')}
                                className="w-5 h-5 flex items-center justify-center rounded text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
                                title="Clear (Esc)"
                                aria-label="Clear filter"
                            >
                                <X className="w-3 h-3" />
                            </button>
                        </div>
                    )}
                </div>
                <button
                    onClick={() => refreshSchema()}
                    className="w-8 h-8 flex items-center justify-center rounded-md hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] flex-shrink-0"
                    title={`Refresh schema (${schemaInfo.database_name})`}
                    aria-label="Refresh schema"
                >
                    <RefreshCw className={`w-3.5 h-3.5 ${schemaLoading ? 'animate-spin' : ''}`} />
                </button>
            </div>

            {/* Tree Content - Overflow handled here */}
            <div className="flex-1 overflow-y-auto pl-1 pr-0 pb-2 min-h-0">
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
