interface SidebarResizerProps {
  orientation: "row" | "col" | "main";
  title: string;
  onPointerDown: (e: React.PointerEvent<HTMLElement>) => void;
}

export function SidebarResizer({
  orientation,
  title,
  onPointerDown,
}: SidebarResizerProps) {
  return (
    <div
      className={`sidebar-resizer sidebar-resizer-${orientation}`}
      role="separator"
      aria-orientation={orientation === "row" ? "horizontal" : "vertical"}
      title={title}
      onPointerDown={onPointerDown}
    />
  );
}
