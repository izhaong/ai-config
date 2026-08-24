import { useEventListener, useMemoizedFn } from "ahooks";
import { useCallback, useEffect, useRef, useState } from "react";

const STORAGE_KEY = "agent-manager.sidebar.layout";

export interface SidebarLayout {
  sidebarW: number;
  projectsRatio: number;
  assetsRatio: number;
}

const DEFAULT_LAYOUT: SidebarLayout = {
  sidebarW: 320,
  projectsRatio: 0.4,
  assetsRatio: 0.5,
};

const LIMITS = {
  sidebarW: { min: 220, max: 560 },
  projectsRatio: { min: 0.18, max: 0.72 },
  assetsRatio: { min: 0.22, max: 0.78 },
} as const;

function clamp(n: number, min: number, max: number) {
  return Math.min(max, Math.max(min, n));
}

function loadLayout(): SidebarLayout {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_LAYOUT;
    const parsed = JSON.parse(raw) as Partial<SidebarLayout>;
    return {
      sidebarW: clamp(
        parsed.sidebarW ?? DEFAULT_LAYOUT.sidebarW,
        LIMITS.sidebarW.min,
        LIMITS.sidebarW.max,
      ),
      projectsRatio: clamp(
        parsed.projectsRatio ?? DEFAULT_LAYOUT.projectsRatio,
        LIMITS.projectsRatio.min,
        LIMITS.projectsRatio.max,
      ),
      assetsRatio: clamp(
        parsed.assetsRatio ?? DEFAULT_LAYOUT.assetsRatio,
        LIMITS.assetsRatio.min,
        LIMITS.assetsRatio.max,
      ),
    };
  } catch {
    return DEFAULT_LAYOUT;
  }
}

function saveLayout(layout: SidebarLayout) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(layout));
  } catch {
    /* ignore quota / private mode */
  }
}

export type SidebarResizeKind = "sidebar" | "projects" | "assets";

export function useSidebarLayout() {
  const [layout, setLayout] = useState<SidebarLayout>(loadLayout);
  const layoutRef = useRef(layout);
  const shellRef = useRef<HTMLElement>(null);
  const [shellSize, setShellSize] = useState({ w: 0, h: 0 });
  const dragRef = useRef<{
    kind: SidebarResizeKind;
    startX: number;
    startY: number;
    snapshot: SidebarLayout;
  } | null>(null);

  useEffect(() => {
    layoutRef.current = layout;
  }, [layout]);

  useEffect(() => {
    const el = shellRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      setShellSize({
        w: entry.contentRect.width,
        h: entry.contentRect.height,
      });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const applyLayout = useMemoizedFn((next: SidebarLayout) => {
    layoutRef.current = next;
    setLayout(next);
  });

  const onPointerMove = useMemoizedFn((e: PointerEvent) => {
    const drag = dragRef.current;
    if (!drag) return;
    const { kind, startX, startY, snapshot } = drag;

    if (kind === "sidebar") {
      applyLayout({
        ...snapshot,
        sidebarW: clamp(
          snapshot.sidebarW + (e.clientX - startX),
          LIMITS.sidebarW.min,
          LIMITS.sidebarW.max,
        ),
      });
      return;
    }

    if (kind === "projects") {
      const shell = shellRef.current;
      const totalH = shell?.clientHeight ?? shellSize.h;
      if (totalH <= 0) return;
      applyLayout({
        ...snapshot,
        projectsRatio: clamp(
          snapshot.projectsRatio + (e.clientY - startY) / totalH,
          LIMITS.projectsRatio.min,
          LIMITS.projectsRatio.max,
        ),
      });
      return;
    }

    const shell = shellRef.current;
    const totalW = shell?.clientWidth ?? shellSize.w;
    if (totalW <= 0) return;
    applyLayout({
      ...snapshot,
      assetsRatio: clamp(
        snapshot.assetsRatio + (e.clientX - startX) / totalW,
        LIMITS.assetsRatio.min,
        LIMITS.assetsRatio.max,
      ),
    });
  });

  const endDrag = useMemoizedFn(() => {
    if (dragRef.current) {
      saveLayout(layoutRef.current);
    }
    dragRef.current = null;
    document.body.classList.remove("sidebar-resizing");
    document.body.style.cursor = "";
  });

  useEventListener("pointermove", onPointerMove, { target: () => document });
  useEventListener("pointerup", endDrag, { target: () => document });
  useEventListener("pointercancel", endDrag, { target: () => document });

  const startResize = useCallback(
    (kind: SidebarResizeKind, e: React.PointerEvent<HTMLElement>) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      dragRef.current = {
        kind,
        startX: e.clientX,
        startY: e.clientY,
        snapshot: { ...layoutRef.current },
      };
      document.body.classList.add("sidebar-resizing");
      document.body.style.cursor =
        kind === "projects" ? "row-resize" : "col-resize";
    },
    [],
  );

  const resizerH = 5;
  const projectsH =
    shellSize.h > 0
      ? Math.round(shellSize.h * layout.projectsRatio - resizerH / 2)
      : undefined;
  const assetsW =
    shellSize.w > 0
      ? Math.round(shellSize.w * layout.assetsRatio - resizerH / 2)
      : undefined;

  return {
    layout,
    shellRef,
    startResize,
    projectsH,
    assetsW,
  };
}
