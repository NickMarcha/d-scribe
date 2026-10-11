import { useEffect, useRef, useState, type ReactNode } from "react";
import "./Menu.css";

/** A button that opens a small panel below it. Closes on outside click or Escape. */
export function Menu({
  label,
  title,
  align = "right",
  children,
}: {
  label: ReactNode;
  title?: string;
  align?: "left" | "right";
  /** Content of the panel. Call `close` after an action to hide it. */
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="menu" ref={ref}>
      <button type="button" className={open ? "active" : ""} onClick={() => setOpen(!open)} title={title} aria-expanded={open}>
        {label}
      </button>
      {open && <div className={`menu-panel menu-panel--${align}`}>{children(() => setOpen(false))}</div>}
    </div>
  );
}
