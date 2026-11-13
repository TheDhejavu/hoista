/**
 * A row's "⋮" menu: the things to do to one row, so a list stays quiet.
 * Each item opens its own dialog or runs at once; a destructive one is red
 * and goes last. The menu floats above the page, so a card's rounded edge
 * never cuts it off, and opens upward near the bottom of the window.
 */
import { MoreVertical } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cx } from '@ui/lib/cx';
import styles from './RowMenu.module.css';

export interface RowMenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
}

interface Place {
  right: number;
  top?: number;
  bottom?: number;
  /** Where the menu goes: the open dialog it's in, so it shares its top layer, or the page. */
  into: Element;
}

export function RowMenu({ label, items }: { label: string; items: RowMenuItem[] }) {
  const [place, setPlace] = useState<Place | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const open = place !== null;

  useEffect(() => {
    if (!open) return;
    function onPointer(event: MouseEvent): void {
      const target = event.target as Node;
      if (!menu.current?.contains(target) && !button.current?.contains(target)) setPlace(null);
    }
    function onKey(event: KeyboardEvent): void {
      if (event.key !== 'Escape') return;
      // Escape closes the menu only, not the dialog it's in.
      event.preventDefault();
      event.stopPropagation();
      setPlace(null);
    }
    function onMove(event: Event): void {
      if (event.target instanceof Node && menu.current?.contains(event.target)) return;
      setPlace(null);
    }
    document.addEventListener('mousedown', onPointer);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onMove);
    document.addEventListener('scroll', onMove, true);
    return () => {
      document.removeEventListener('mousedown', onPointer);
      document.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onMove);
      document.removeEventListener('scroll', onMove, true);
    };
  }, [open]);

  function toggle(): void {
    if (open || !button.current) {
      setPlace(null);
      return;
    }
    const box = button.current.getBoundingClientRect();
    const right = window.innerWidth - box.right;
    // Room for the items below, or open upward.
    const below = window.innerHeight - box.bottom > 40 * items.length + 24;
    const into = button.current.closest('dialog') ?? document.body;
    setPlace(
      below
        ? { right, top: box.bottom + 4, into }
        : { right, bottom: window.innerHeight - box.top + 4, into },
    );
  }

  if (items.length === 0) return null;
  return (
    <>
      <button
        ref={button}
        type="button"
        className={styles.more}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggle}
      >
        <MoreVertical size={16} aria-hidden="true" />
      </button>
      {place &&
        createPortal(
          <div
            ref={menu}
            className={styles.menu}
            role="menu"
            style={{ right: place.right, top: place.top, bottom: place.bottom }}
          >
            {items.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                disabled={item.disabled}
                className={cx(item.danger && styles.danger)}
                onClick={() => {
                  setPlace(null);
                  item.onSelect();
                }}
              >
                {item.label}
              </button>
            ))}
          </div>,
          place.into,
        )}
    </>
  );
}
