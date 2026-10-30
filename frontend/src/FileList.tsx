// frontend/src/FileList.tsx
// Deps: @tanstack/react-virtual, @tauri-apps/api, @crabnebula/tauri-plugin-drag
// Contract: this component NEVER holds more than a few pages of rows.
// total comes from the window count in list_files (search/src/lib.rs).
import { useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import { startDrag } from "@crabnebula/tauri-plugin-drag";

type Row = {
  id: number;
  rel_path: string;
  name: string;
  ext: string;
  duration_ms: number | null;
  bpm: number | null;
  favorite: boolean;
};
type Page = { rows: Row[]; total: number };

const PAGE = 200;

export function FileList(props: {
  query: { q: string; tags: number[]; fav: boolean };
  onPreview: (id: number) => void;
}) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [total, setTotal] = useState(0);
  const [pages, setPages] = useState<Map<number, Page>>(new Map());
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const loaded = useRef<Set<number>>(new Set());

  const fetchPage = useCallback(
    async (offset: number) => {
      if (loaded.current.has(offset)) return;
      loaded.current.add(offset);
      const page = await invoke<Page>("list_files", {
        dto: {
          q: props.query.q || null,
          tags_include: props.query.tags,
          favorites_only: props.query.fav,
          limit: PAGE,
          offset,
        },
      });
      setPages((prev) => {
        const next = new Map(prev);
        next.set(offset, page);
        return next;
      });
      setTotal(page.total);
    },
    [props.query]
  );

  // query change = full reset (loaded cache keyed by offset is per-query)
  useEffect(() => {
    loaded.current.clear();
    setPages(new Map());
    setSelected(new Set());
    fetchPage(0);
  }, [props.query, fetchPage]);

  const getRow = (index: number): Row | undefined => {
    const base = Math.floor(index / PAGE) * PAGE;
    const page = pages.get(base);
    if (!page) {
      void fetchPage(base);
      return undefined;
    }
    return page.rows[index - base];
  };

  const virt = useVirtualizer({
    count: total,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 34,
    overscan: 20,
  });

  const onRowClick = (e: React.MouseEvent, row: Row) => {
    props.onPreview(row.id);
    setSelected((prev) => {
      const next = new Set(e.ctrlKey || e.metaKey ? prev : []);
      if (next.has(row.id)) next.delete(row.id);
      else next.add(row.id);
      return next;
    });
  };

  // IMPORTANT: native drag, not HTML5 dnd (see spikes/drag-out/README.md)
  const pointerStart = useRef<{ x: number; y: number; id: number } | null>(null);
  const onPointerDown = (e: React.PointerEvent, row: Row) => {
    if (e.button !== 0) return;
    pointerStart.current = { x: e.clientX, y: e.clientY, id: row.id };
  };
  const onPointerMove = async (e: React.PointerEvent) => {
    const start = pointerStart.current;
    if (!start) return;
    if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 6) return;
    pointerStart.current = null;
    const ids = selected.has(start.id) ? Array.from(selected) : [start.id];
    const paths = await invoke<string[]>("paths_for_ids", { ids });
    if (paths.length > 0) {
      startDrag({ item: paths, icon: "" }).catch((err) =>
        console.error("startDrag failed", err)
      );
    }
  };

  return (
    <div ref={parentRef} className="file-list" onPointerMove={onPointerMove}>
      <div style={{ height: virt.getTotalSize(), position: "relative" }}>
        {virt.getVirtualItems().map((vi) => {
          const row = getRow(vi.index);
          return (
            <div
              key={vi.key}
              className={
                row && selected.has(row.id) ? "file-row selected" : "file-row"
              }
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: vi.size,
                transform: "translateY(" + vi.start + "px)",
              }}
              onPointerDown={row ? (e) => onPointerDown(e, row) : undefined}
              onClick={row ? (e) => onRowClick(e, row) : undefined}
            >
              {row ? (
                <span>
                  <b>{row.name}</b>
                  <i>{row.ext}</i>
                  <em>{row.duration_ms ? Math.round(row.duration_ms / 1000) + "s" : ""}</em>
                  <em>{row.bpm ? Math.round(row.bpm) + " bpm" : ""}</em>
                </span>
              ) : (
                <span className="row-skeleton" />
              )}
            </div>
          );
        })}
      </div>
      <footer className="file-count">{total.toLocaleString()} results</footer>
    </div>
  );
}
