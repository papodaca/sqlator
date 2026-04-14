---
title: "feat: Map Visualization for Query Results — Leaflet Geo Rendering in New Tab"
type: feat
status: active
date: 2026-04-14
---

# feat: Map Visualization for Query Results — Leaflet Geo Rendering in New Tab

## Overview

When a SQL query (or table browse) returns geographic data — lat/lon coordinate pairs, WKT geometry strings, or GeoJSON — an "Open Map" button appears in the result pane. Clicking it opens a new query tab containing a full Leaflet map with the data plotted. The map tab retains the source SQL and connection so it can be refreshed independently. Tile sets are sourced from CARTO (Positron, Dark Matter, Voyager). Point data supports a heatmap toggle in addition to marker mode. Mixed geometry types (points, lines, polygons) each render with their appropriate Leaflet layer. This is a spatial data exploration aid, not a GIS system.

## Problem Statement / Motivation

SQL databases — especially PostGIS, SpatiaLite, and any table with `lat`/`lon` columns — frequently hold geographic data. Reading raw coordinate tuples in a grid gives no spatial insight. Today sqlator users copy-paste results into QGIS, kepler.gl, or Excel to see a map. An inline map tab closes that loop: one click from a query result to a live interactive map. The tab model means the editor stays open alongside the map, and the refresh button keeps the map current as the query evolves.

## Proposed Solution

**Detection:** After a query resolves to `result.kind === "results"`, a `detectGeoColumns()` function (in `src/lib/services/geo-detect.ts`) scans `result.columns` and up to 20 non-null sampled values per column. If a geo-capable configuration is found, an "Open Map" button appears in `ResultPane.svelte` near the row-count / export controls.

**Tab opening:** Clicking "Open Map" calls `tabs.openMapView(connectionId, sourceSql, result.columns, result.rows, geoConfig)` in `tabs.svelte.ts`. This creates a new `QueryTab` with `mapView?: MapViewState` set — following the same discriminated union pattern used by `tableBrowse` and `schemaDdl`.

**Map rendering:** `TabbedEditor.svelte` gains a `{:else if activeQueryTab.mapView}` branch that mounts `<MapTab>`. `MapTab.svelte` initializes Leaflet inside `onMount`, renders the appropriate layer (markers, `L.geoJSON`, or heatmap), and fits the map to the data's bounding box. Tile sets default to match the app theme (Dark Matter for dark mode, Positron for light). A toolbar offers tile-set picker and heatmap/markers toggle.

**Refresh:** The map tab stores `sourceSql` and `connectionId`. A refresh button re-executes the SQL, re-runs geo detection, and re-renders the map with updated data — no source tab required.

## Technical Approach

### Architecture

```
ResultPane.svelte
  └── detectGeoColumns() → shows "Open Map" button
        └── tabs.openMapView()
              └── new QueryTab { mapView: MapViewState }
                    └── TabbedEditor.svelte branch
                          └── MapTab.svelte
                                ├── onMount: L.map(container), tile layer, geo layer
                                ├── toolbar: tile picker, render mode toggle, refresh
                                └── $effect: swap tile layer / geo layer reactively
```

**No changes to ResultPane props, QueryTab result shape, or IPC commands.** All geo logic is client-side.

### New Types (`src/lib/types.ts`)

```ts
export type CartoTileLayer = 'positron' | 'dark-matter' | 'voyager';
export type MapRenderMode = 'markers' | 'heatmap';

export type GeoConfigKind = 'lat-lon' | 'wkt' | 'geojson';

export interface GeoConfig {
  kind: GeoConfigKind;
  latCol?: string;      // lat-lon only
  lonCol?: string;      // lat-lon only
  geoCol?: string;      // wkt or geojson only
  labelCols: string[];  // non-geo columns shown in popups
}

export interface MapViewState {
  connectionId: string;
  sourceSql: string;                      // self-contained; source tab may be closed
  columns: string[];
  rows: Record<string, unknown>[];
  geoConfig: GeoConfig;
  tileLayer: CartoTileLayer;
  renderMode: MapRenderMode;
  isRefreshing: boolean;
}
```

`QueryTab` gains one new optional field:
```ts
mapView?: MapViewState;   // present → map view mode (no tableBrowse or schemaDdl)
```

### Geo Detection Algorithm (`src/lib/services/geo-detect.ts`)

Priority order (first match wins):

1. **Explicit geometry column name**: column named `geom`, `geometry`, `shape`, `wkt`, `the_geom`, `geo` — probe first 20 non-null values for WKT prefix (`POINT`, `LINESTRING`, `POLYGON`, `MULTI*`, `GEOMETRYCOLLECTION`) or JSON `type` key.
2. **GeoJSON column name**: column named `geojson`, `geo_json`, `feature`, `location` — probe first 20 non-null values for a JSON object/string with a `type` key matching a GeoJSON geometry type.
3. **Lat/lon column name pair**: look for a latitude column (`lat`, `latitude`) AND a longitude column (`lon`, `lng`, `longitude`) — both must be numeric in the sample; values must fall within `[-90,90]` × `[-180,180]`.
4. **WKT value sampling** (fallback): for any string column with a name not matching the above, probe first 20 non-null values for WKT prefix.

`x`/`y` column names are intentionally excluded from auto-detection (they may be projected coordinates, not lat/lon). 

```ts
// src/lib/services/geo-detect.ts
export function detectGeoColumns(
  columns: string[],
  rows: Record<string, unknown>[]
): GeoConfig | null { ... }

const WKT_PREFIXES = ['POINT', 'LINESTRING', 'POLYGON', 'MULTIPOINT',
  'MULTILINESTRING', 'MULTIPOLYGON', 'GEOMETRYCOLLECTION'];

function sampleNonNull(col: string, rows: Record<string, unknown>[], n = 20): unknown[] {
  const out: unknown[] = [];
  for (const r of rows) {
    if (r[col] !== null && r[col] !== undefined && out.length < n) out.push(r[col]);
    if (out.length >= n) break;
  }
  return out;
}
```

### Tile URLs (CARTO)

| Layer | URL |
|-------|-----|
| Positron (light) | `https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png` |
| Dark Matter (dark) | `https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png` |
| Voyager (neutral) | `https://{s}.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}.png` |

Subdomains: `abcd`. Max zoom: 20. Required attribution:
```
© OpenStreetMap contributors © CARTO
```

Default tile selection: `theme.isDark ? 'dark-matter' : 'positron'` (read once at tab open; user may override).

### Leaflet Initialization Pattern (`MapTab.svelte`)

```ts
import { onMount } from 'svelte';
import L from 'leaflet';
import 'leaflet/dist/leaflet.css';
// Fix Vite bundler marker icon path resolution
import markerIcon2x from 'leaflet/dist/images/marker-icon-2x.png';
import markerIcon from 'leaflet/dist/images/marker-icon.png';
import markerShadow from 'leaflet/dist/images/marker-shadow.png';
delete (L.Icon.Default.prototype as any)._getIconUrl;
L.Icon.Default.mergeOptions({ iconRetinaUrl: markerIcon2x, iconUrl: markerIcon, shadowUrl: markerShadow });

let mapContainer: HTMLDivElement;
let map: L.Map | undefined;

onMount(() => {
  map = L.map(mapContainer);
  // ... add tile layer, geo layer, fit bounds
  const ro = new ResizeObserver(() => map?.invalidateSize({ animate: false }));
  ro.observe(mapContainer);
  return () => { ro.disconnect(); map?.remove(); map = undefined; };
});
```

Reactive tile/layer swaps happen in a `$effect` that guards `if (!map) return` — never re-calls `L.map()`.

### Geo Layer Rendering

- **lat-lon → markers**: iterate `rows`, skip null/invalid coords, call `L.marker([lat, lon]).bindPopup(popupHtml)`.
- **WKT → `L.geoJSON`**: parse each cell with `wellknown.parse(wktString)`, collect into a `FeatureCollection`, pass to `L.geoJSON(fc, { onEachFeature })`.
- **GeoJSON column**: parse cell value (string or object), detect if it is a single geometry or a `FeatureCollection`, pass to `L.geoJSON`.
- **Mixed geometry types in one WKT/GeoJSON column**: per-row branching — each feature is added to the same `L.geoJSON` layer regardless of geometry type. Points get default marker style, lines get stroke, polygons get fill.
- **Heatmap mode** (lat-lon only): replace marker layer with `L.heatLayer(points, { radius: 25 })` from `@linkurious/leaflet-heat`. The heatmap toggle is hidden for WKT/GeoJSON render modes.

After rendering, call `map.fitBounds(layer.getBounds(), { padding: [20, 20] })`.

### Popup Content

For lat-lon markers and each WKT/GeoJSON feature, construct a simple HTML popup from `geoConfig.labelCols`:
```ts
function buildPopupHtml(row: Record<string, unknown>, labelCols: string[]): string {
  if (!labelCols.length) return '';
  return '<table>' + labelCols.map(col =>
    `<tr><th>${col}</th><td>${row[col] ?? ''}</td></tr>`
  ).join('') + '</table>';
}
```
No `innerHTML` from unsanitized data is injected elsewhere; Leaflet's `.bindPopup(html)` is the only dynamic HTML. Column names and values come from query results — XSS risk is limited to self-inflicted by the user's own SQL, which is acceptable in a local desktop app.

### Refresh Flow

1. User clicks "Refresh" in map toolbar
2. `mapView.isRefreshing = true` (shows spinner)
3. Call existing `execute_query` Tauri command with `mapView.connectionId` and `mapView.sourceSql`
4. On result: re-run `detectGeoColumns()` on new columns/rows
   - If geo config still valid: update `mapView.rows`, `mapView.columns`, re-render
   - If geo columns gone: show empty-state message: "Query no longer returns mappable data"
5. `mapView.isRefreshing = false`

No new Tauri IPC command needed — reuse `execute_query`.

### Table Browse "Open Map"

When `activeQueryTab.tableBrowse` is set and geo columns are detected in the loaded rows, show an "Open Map" button in the table browse toolbar. Clicking it constructs `sourceSql = "SELECT * FROM \`${schema}\`.\`${tableName}\`"` (or `schema.tableName` for Postgres) and calls `tabs.openMapView()` with the currently-loaded rows and that synthetic SQL. Refresh re-executes the full `SELECT *` to get an updated snapshot.

### `tabs.openMapView()` (`src/lib/stores/tabs.svelte.ts`)

```ts
function openMapView(
  connectionId: string,
  sourceSql: string,
  columns: string[],
  rows: Record<string, unknown>[],
  geoConfig: GeoConfig
): void {
  const isDark = theme.isDark;  // read current theme
  const newTab: QueryTab = {
    id: crypto.randomUUID(),
    label: `Map: ${activeQueryTab?.label ?? 'Query'}`,
    sql: '',
    isDirty: false,
    result: { kind: 'idle' },
    isExecuting: false,
    mapView: {
      connectionId,
      sourceSql,
      columns,
      rows,
      geoConfig,
      tileLayer: isDark ? 'dark-matter' : 'positron',
      renderMode: 'markers',
      isRefreshing: false,
    },
  };
  queryTabs = [...queryTabs, newTab];
  activeQueryTabId = newTab.id;
}
```

No deduplication check for map tabs (unlike `openTableBrowse`/`openSchemaDdl`) — the user may open multiple maps from different queries.

### `TabbedEditor.svelte` Branch

In the `{:else if activeQueryTab.tableBrowse}` / `{:else if activeQueryTab.schemaDdl}` chain, add:
```svelte
{:else if activeQueryTab.mapView}
  <MapTab
    mapView={activeQueryTab.mapView}
    onMapViewChange={(updated) => tabs.updateMapView(activeQueryTab.id, updated)}
  />
```

`tabs.updateMapView(tabId, partial)` merges partial updates into `mapView` — used for tile layer changes, render mode toggle, and refresh state.

### Tab Persistence

`tabs.svelte.ts` currently persists `id`, `label`, `sql`, `tableBrowse`, `schemaDdl`. Add `mapView` to the serialized shape. On restore, set `mapView.isRefreshing = false` — the map re-renders from the persisted `rows`/`columns` without a fresh query. If `rows` is empty on restore (unlikely but possible), show the empty-state message.

**Rows in localStorage:** Map tabs persist `rows` and `columns` in `localStorage` via the existing tab persistence mechanism. For large result sets this may hit localStorage limits (~5MB). Mitigate by capping persisted rows at 5,000. If `rows.length > 5000`, persist a truncated copy and a `wasTruncated: true` flag; restore the map with the truncated data and a "Showing first 5,000 rows — refresh to reload" notice.

### CSP (`src-tauri/tauri.conf.json`)

Current `"csp": null` — Tauri injects no policy. Tile fetching works on all platforms (CARTO is HTTPS; Tauri v2 on macOS/Windows/Linux allows HTTPS image fetches with no CSP). No immediate change required for v1.

**Recommended for production hardening** (add to `tauri.conf.json`):
```json
"security": {
  "csp": {
    "default-src": "'self' ipc: http://ipc.localhost",
    "script-src": "'self'",
    "style-src": "'self' 'unsafe-inline'",
    "img-src": "'self' asset: data: blob: https://*.basemaps.cartocdn.com",
    "connect-src": "ipc: http://ipc.localhost",
    "font-src": "'self'"
  }
}
```

`'unsafe-inline'` in `style-src` is required because Leaflet injects inline styles for marker positioning.

### Performance

- **Point cap**: Render up to 10,000 markers. If `rows.length > 10,000`, show a notice "Showing first 10,000 rows" and truncate before rendering. The heatmap layer handles density natively via `@linkurious/leaflet-heat` and does not need a hard cap.
- **WKT/GeoJSON**: No per-feature cap; Leaflet's `L.geoJSON` layer handles large feature counts better than individual markers. Add a loading spinner during the initial render pass.
- **`map.invalidateSize`**: Called on `ResizeObserver` with `{ animate: false }` to handle panel resizing.

## New Dependencies

```
pnpm add leaflet wellknown @linkurious/leaflet-heat
pnpm add -D @types/leaflet @types/wellknown
```

| Package | Version (approx) | Purpose |
|---------|-----------------|---------|
| `leaflet` | `^1.9.x` | Map rendering, tile layers, markers, geoJSON |
| `wellknown` | `^0.5.x` | WKT string → GeoJSON geometry parsing |
| `@linkurious/leaflet-heat` | `^0.2.6` | Maintained fork of `leaflet.heat`; heatmap layer |
| `@types/leaflet` | devDep | TypeScript types for Leaflet |
| `@types/wellknown` | devDep | TypeScript types for wellknown |

`@linkurious/leaflet-heat` has no TypeScript types package; add a minimal ambient declaration:
```ts
// src/lib/types/leaflet-heat.d.ts
import * as L from 'leaflet';
declare module 'leaflet' {
  function heatLayer(latlngs: [number, number, number?][], options?: HeatLayerOptions): HeatLayer;
  interface HeatLayerOptions { minOpacity?: number; radius?: number; blur?: number; max?: number; }
  interface HeatLayer extends L.Layer { setLatLngs(latlngs: [number, number, number?][]): this; }
}
```

## System-Wide Impact

### Interaction Graph

"Open Map" click → `tabs.openMapView()` → `queryTabs` `$state` mutation → `TabbedEditor.svelte` `$derived` re-evaluates `activeQueryTab` → `<MapTab>` mounts → `onMount` fires → `L.map()` initializes. No Tauri IPC on open.

"Refresh" click → `execute_query` IPC (existing command) → result stream → `tabs.updateMapView()` → `MapTab` `$effect` re-renders geo layer. Same execution path as a normal query, no new commands.

### Error Propagation

- **Geo detection failure** (all rows null, unparseable values): returns `null` from `detectGeoColumns()` → "Open Map" button does not appear.
- **WKT parse error** on a row: `wellknown.parse()` returns `null` for malformed WKT. Skip that row silently; increment a `skippedCount` shown in the toolbar as "N rows skipped (invalid geometry)".
- **Leaflet rendering error**: wrap `map.fitBounds()` and layer addition in try/catch inside `onMount`. On catch, display an error banner inside `MapTab` without affecting the source result pane.
- **Tile load failure** (offline): Leaflet shows a gray tile placeholder. Add an `L.control` overlay: "Tiles unavailable (offline?)" that appears if > 5 tile error events fire within 3 seconds.
- **Refresh IPC error**: set `mapView.isRefreshing = false`, show an inline error banner above the map.

### State Lifecycle Risks

- **Source tab closed**: map tab is fully self-contained (`sourceSql` + `connectionId` stored on `mapView`). Refresh works regardless of source tab state.
- **Connection closed/removed**: refresh will produce an `execute_query` IPC error. Show "Connection unavailable — reconnect to refresh" inline.
- **`closeAllQueryTabs`** (`tabs.svelte.ts:255`): destroys map tabs along with all others. Leaflet teardown fires via `onMount` cleanup. No orphaned state.
- **Map container unmount without `map.remove()`**: guarded by `onMount` return cleanup. The `ResizeObserver` is also disconnected in the same cleanup.

### API Surface Parity

No new Tauri commands. Map refresh reuses `execute_query` (same path as the query editor). No backend changes.

### Integration Test Scenarios

1. Query returning `lat`, `lon`, `city_name` — "Open Map" button appears, opens map tab with markers, popup shows `city_name`.
2. Query where `lat` is `null` for first 5 rows but numeric in rows 6–20 — geo detection still fires (samples up to 20 non-null values).
3. WKT column with mixed `POINT` and `POLYGON` rows — both render in the same `L.geoJSON` layer on one map.
4. Refresh after the source query tab is closed — re-executes SQL successfully using stored `connectionId`.
5. Open map, switch app to dark mode — tile layer remains as user-selected (no forced re-tile on theme change after initial open).

## Acceptance Criteria

### Geo Detection

- [ ] "Open Map" button appears when `result.kind === "results"` and `detectGeoColumns()` returns non-null
- [ ] Button is hidden (not disabled) when no geo columns are found
- [ ] Detection samples up to 20 non-null values per column (not just `rows[0]`)
- [ ] Column order in `labelCols` reflects `result.columns` array order
- [ ] Columns named `x`/`y` alone do NOT trigger geo detection
- [ ] WKT detection requires a WKT-prefixed string (`POINT`, `POLYGON`, etc.) in sampled values, not just a string column

### Map Tab

- [ ] Clicking "Open Map" opens a new query tab labeled `Map: [source tab label]`
- [ ] Map tab renders a Leaflet map (not a blank page)
- [ ] Map fits to the data's bounding box on initial load
- [ ] Toolbar contains: tile-set picker (Positron / Dark Matter / Voyager), render-mode toggle (Markers / Heatmap), Refresh button
- [ ] Heatmap toggle is only shown for `lat-lon` geo configs (not WKT/GeoJSON)
- [ ] Default tile matches app theme at the time the tab is opened
- [ ] Tile switches without losing pan/zoom position
- [ ] Render mode switch between markers and heatmap does not require page reload
- [ ] Map remains functional after the source query tab is closed

### Rendering

- [ ] Lat/lon data renders as clickable markers with popup showing label columns
- [ ] WKT `POINT` values render as markers
- [ ] WKT `POLYGON` / `LINESTRING` values render as `L.geoJSON` overlays
- [ ] GeoJSON cell values (single geometry or FeatureCollection) render correctly
- [ ] Rows with null or invalid coordinates are skipped; a count of skipped rows appears in toolbar if any
- [ ] Point cap of 10,000 rows enforced; notice shown when truncating

### Refresh

- [ ] Refresh button re-executes `sourceSql` on the stored `connectionId`
- [ ] Map updates with new data after refresh
- [ ] If refreshed result has no geo columns, an empty-state message replaces the map
- [ ] Refresh spinner shows while executing; clears on completion or error

### Table Browse

- [ ] "Open Map" button appears in table browse toolbar when loaded rows contain geo columns
- [ ] Map tab for table browse stores `SELECT * FROM schema.table` as `sourceSql`
- [ ] Refresh from table browse map re-executes the synthetic `SELECT *`

### Technical

- [ ] Leaflet map container resizes correctly when the panel is resized
- [ ] No Leaflet "Map container is already initialized" error when switching tabs and returning
- [ ] Map is destroyed (`.remove()`) when tab is closed — no memory leak
- [ ] No new Tauri IPC commands added
- [ ] `leaflet/dist/leaflet.css` is imported and markers display correctly (no broken image icons)
- [ ] CARTO tile attribution is rendered in the map's attribution control

### Resilience

- [ ] A WKT parse failure on one row does not crash the map — row is skipped
- [ ] A Leaflet rendering error does not crash `ResultPane` or the query editor
- [ ] Tile load failures (offline) surface a notice; the map and markers are still accessible

## Dependencies & Risks

| Risk | Mitigation |
|------|-----------|
| Leaflet double-init on tab switch | `onMount` return cleanup calls `map.remove()`; `bind:this` (not string ID) avoids stale `_leaflet_id`; `{#key activeQueryTab.id}` on `<MapTab>` forces fresh container |
| `@linkurious/leaflet-heat` TypeScript types absent | Provide minimal ambient declaration in `src/lib/types/leaflet-heat.d.ts` |
| Vite bundles Leaflet marker PNGs incorrectly | Use `import markerIcon from 'leaflet/dist/images/marker-icon.png'` + `L.Icon.Default.mergeOptions` (documented fix) |
| 50k rows from unified-grid plan freezing Leaflet | Hard cap at 10,000 rendered points; heatmap mode bypasses this for density visualization |
| `localStorage` overflow from large persisted `rows` | Cap persisted rows at 5,000; show "Showing first 5,000 rows — refresh to reload" on restore |
| CARTO tiles blocked in strict CSP environments | `"csp": null` in current config means no issue; document explicit CSP snippet for production hardening |
| `wellknown` parses 2D WKT only; PostGIS 3D (Z) coords common | `wellknown` handles Z coordinates (3D) — `POINT Z (x y z)` parses correctly to a 3D GeoJSON coordinate |
| `theme.isDark` not available in `tabs.openMapView()` | Read `theme.isDark` at call site in the component and pass default tile as argument |

## Success Metrics

- "Open Map" workflow works end-to-end on all three database types (SQLite with SpatiaLite, Postgres/PostGIS, MySQL with `lat`/`lon` columns)
- WKT geometry rendering works for at least `POINT`, `POLYGON`, `LINESTRING` variants
- No regressions in `ResultPane` for non-geo results (empty, error, rowsAffected)
- Map tab lifecycle (open, switch away, switch back, close) produces no console errors

## Files to Create / Modify

### New

- `src/lib/services/geo-detect.ts` — geo column detection utility (`detectGeoColumns`, `sampleNonNull`, WKT prefix check, GeoJSON probe)
- `src/lib/components/MapTab.svelte` — full Leaflet map tab: toolbar (tile picker, mode toggle, refresh), map container, geo layer rendering
- `src/lib/types/leaflet-heat.d.ts` — ambient TypeScript declaration for `@linkurious/leaflet-heat`

### Modified

- `src/lib/types.ts` — add `CartoTileLayer`, `MapRenderMode`, `GeoConfig`, `GeoConfigKind`, `MapViewState`; add `mapView?: MapViewState` to `QueryTab`
- `src/lib/stores/tabs.svelte.ts` — add `openMapView()`, `updateMapView()`, update `PersistedTabState` to include `mapView`
- `src/lib/components/TabbedEditor.svelte` — add `{:else if activeQueryTab.mapView}` branch mounting `<MapTab>`
- `src/lib/components/ResultPane.svelte` — add "Open Map" button in the `result.kind === "results"` block
- `package.json` — add `leaflet`, `wellknown`, `@linkurious/leaflet-heat` (+ type devDeps)

### Optionally Modified

- `src-tauri/tauri.conf.json` — add explicit CSP with `https://*.basemaps.cartocdn.com` in `img-src` for production hardening
- Table browse component (whichever component renders the table browse toolbar) — add "Open Map" button

### Unchanged

- `core/` (Rust) — no backend changes
- `tui-app/` — map visualization deferred to a follow-up
- `src/app.css` — no new global styles required; `MapTab.svelte` uses scoped styles

## Sources & References

- **Tab discriminated union pattern:** `src/lib/stores/tabs.svelte.ts` (`openTableBrowse`, `openSchemaDdl`)
- **QueryTab interface:** `src/lib/types.ts:115–124`
- **ResultPaneState union:** `src/lib/types.ts:94–106`
- **ResultPane insertion point:** `src/lib/components/ResultPane.svelte:73–89`
- **TabbedEditor tab branch pattern:** `src/lib/components/TabbedEditor.svelte:96–152`
- **`$effect` + `$state` over `$derived` gotcha:** commit `2d27e51`
- **Charting plan (parallel reference):** `docs/plans/2026-04-14-007-feat-result-set-charting-plan.md`
- **Leaflet in Svelte 5:** `onMount` for init, return for cleanup; `$effect` only for reactive swaps on existing `map` instance
- **Leaflet marker icon Vite fix:** delete `_getIconUrl`, `L.Icon.Default.mergeOptions({ iconUrl, ... })`
- **CARTO tile URLs:** `https://{s}.basemaps.cartocdn.com/{light_all,dark_all,rastertiles/voyager}/{z}/{x}/{y}.png`, subdomains `abcd`
- **`@linkurious/leaflet-heat`:** maintained fork of `leaflet.heat` (last published ~late 2025)
- **`wellknown`** by Mapbox: WKT → GeoJSON, handles 2D + 3D, ~3KB, ISC license
- **Tauri v2 CSP config:** `src-tauri/tauri.conf.json` `security.csp`; current value is `null` (permissive) — no immediate blocker
