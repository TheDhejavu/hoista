/**
 * Dropdowns. Every choice in the app is a labelled dropdown, never a row of
 * toggle buttons.
 *
 * Options can hold any value (a string, a number, null, an object): the DOM
 * only ever sees each option's position in the list. Pass `isEqual` when the
 * value is an object that won't be the same instance as an option's.
 */
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown } from 'lucide-react';
import { cx } from '@ui/lib/cx';
import styles from './Select.module.css';

/** One choice in a dropdown. Options with the same `group` are listed under that heading. */
export interface Option<T> {
  value: T;
  label: string;
  /** A quieter word after the label, such as which providers a project is on. */
  note?: string;
  group?: string;
  /** Shown but can't be picked, for example a provider that won't run this resource. */
  disabled?: boolean;
}

interface SelectProps<T> {
  value: T;
  options: readonly Option<T>[];
  onChange: (value: T) => void;
  isEqual?: (a: T, b: T) => boolean;
  disabled?: boolean;
  /** A small logo before the choice in the closed dropdown, such as the chosen tool's. */
  icon?: ReactNode;
}

// How an open list is placed, in pixels.
/** Room below the field that's enough to open downwards. */
const LIST_ROOM = 260;
/** Room kept for the list's width at the window's right edge. */
const LIST_WIDTH = 240;
const MAX_LIST_HEIGHT = 360;
/** Space between the field and its list. */
const GAP = 6;
/** Space kept between the list and the window's edge. */
const EDGE = 18;

/**
 * The menu every dropdown opens: our own list in the project's look, not the
 * system's. A button shows the choice; the list opens under it, keeps
 * keyboard behaviour (arrows, Enter, Escape, typing a letter), and closes on
 * a pick, an outside click, scrolling or resizing.
 */
function Dropdown<T>({
  id,
  label,
  value,
  options,
  onChange,
  isEqual = Object.is,
  disabled = false,
  icon,
  className,
  children,
}: SelectProps<T> & {
  id?: string;
  label?: string;
  className?: string;
  /** What the closed dropdown shows; the chosen label when left out. */
  children?: ReactNode;
}) {
  const selected = options.findIndex((option) => isEqual(option.value, value));
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(selected);
  const [place, setPlace] = useState<{
    left: number;
    top: number;
    minWidth: number;
    up: boolean;
    /** How tall the list may be and still fit in the window. */
    room: number;
    host: Element;
  }>();
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const listId = useId();

  const show = () => {
    const box = button.current?.getBoundingClientRect();
    if (!box) return;
    const roomBelow = window.innerHeight - box.bottom;
    // Open downwards when the list fits there; else towards the side with
    // more room, never past the window's edge, so every option can be reached.
    const up = roomBelow <= LIST_ROOM && box.top > roomBelow;
    setPlace({
      left: Math.min(box.left, window.innerWidth - LIST_WIDTH),
      top: up ? box.top - GAP : box.bottom + GAP,
      minWidth: box.width,
      up,
      room: (up ? box.top : roomBelow) - EDGE,
      host: button.current?.closest('dialog') ?? document.body,
    });
    setActive(selected >= 0 ? selected : firstEnabled(options, 0, 1));
    setOpen(true);
  };
  const close = (focus = true) => {
    setOpen(false);
    if (focus) button.current?.focus();
  };
  const pick = (index: number) => {
    const chosen = options[index];
    if (!chosen || chosen.disabled) return;
    close();
    if (index !== selected) onChange(chosen.value);
  };

  useEffect(() => {
    if (!open) return;
    const away = (event: Event) => {
      const target = event.target as Node;
      if (list.current?.contains(target) || button.current?.contains(target)) return;
      setOpen(false);
    };
    const shut = () => {
      setOpen(false);
    };
    // Scrolling the list itself keeps it open; scrolling the page closes it.
    const scrolled = (event: Event) => {
      if (list.current?.contains(event.target as Node)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', away);
    window.addEventListener('resize', shut);
    window.addEventListener('scroll', scrolled, true);
    return () => {
      document.removeEventListener('mousedown', away);
      window.removeEventListener('resize', shut);
      window.removeEventListener('scroll', scrolled, true);
    };
  }, [open]);

  // The list sits in the top layer, so it shows above a modal <dialog> and
  // isn't clipped by scrolling containers.
  useEffect(() => {
    const element = list.current;
    if (!open || !element || typeof element.showPopover !== 'function') return;
    element.showPopover();
    return () => {
      if (element.isConnected) element.hidePopover();
    };
  }, [open, place]);

  useEffect(() => {
    if (open)
      list.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' });
  }, [open, active]);

  const onKey = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
        event.preventDefault();
        show();
      }
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      setActive((now) => firstEnabled(options, now + step, step, now));
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      pick(active);
    } else if (event.key === 'Tab') {
      close(false);
    } else if (event.key.length === 1) {
      const letter = event.key.toLowerCase();
      const found = options.findIndex(
        (option, index) =>
          index > active && !option.disabled && option.label.toLowerCase().startsWith(letter),
      );
      const wrapped =
        found >= 0
          ? found
          : options.findIndex(
              (option) => !option.disabled && option.label.toLowerCase().startsWith(letter),
            );
      if (wrapped >= 0) setActive(wrapped);
    }
  };

  return (
    <>
      <button
        ref={button}
        id={id}
        type="button"
        role="combobox"
        aria-label={id ? undefined : label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open && active >= 0 ? `${listId}-${String(active)}` : undefined}
        className={className}
        disabled={disabled}
        onClick={() => {
          if (open) close();
          else show();
        }}
        onKeyDown={onKey}
      >
        {children ?? (
          <>
            {icon && <span className={styles.icon}>{icon}</span>}
            <span className={styles.value}>
              {options[selected]?.label ?? ''}
              {options[selected]?.note && (
                <span className={styles.note}> · {options[selected].note}</span>
              )}
            </span>
            <ChevronDown size={13} aria-hidden="true" className={styles.chevron} />
          </>
        )}
      </button>
      {open &&
        place &&
        createPortal(
          <div
            ref={list}
            id={listId}
            role="listbox"
            popover="manual"
            aria-label={label}
            className={styles.menu}
            style={{
              left: place.left,
              minWidth: place.minWidth,
              maxHeight: Math.min(MAX_LIST_HEIGHT, place.room),
              ...(place.up ? { bottom: window.innerHeight - place.top } : { top: place.top }),
            }}
          >
            {groupsOf(options).map(({ group, entries }, run) => (
              <div
                key={group ?? `run-${String(run)}`}
                role={group ? 'group' : undefined}
                aria-label={group}
              >
                {group && <div className={styles.menuGroup}>{group}</div>}
                {entries.map(({ label: text, note, index, disabled: off }) => (
                  <div
                    key={`${text}-${String(index)}`}
                    id={`${listId}-${String(index)}`}
                    role="option"
                    aria-selected={index === selected}
                    aria-disabled={off || undefined}
                    data-active={index === active}
                    className={cx(
                      styles.menuItem,
                      index === active && styles.menuActive,
                      off && styles.menuOff,
                    )}
                    onMouseEnter={() => {
                      if (!off) setActive(index);
                    }}
                    onMouseDown={(event) => {
                      event.preventDefault();
                    }}
                    onClick={() => {
                      pick(index);
                    }}
                  >
                    <span>
                      {text}
                      {note && <span className={styles.note}> · {note}</span>}
                    </span>
                    {index === selected && (
                      <Check size={14} aria-hidden="true" className={styles.menuCheck} />
                    )}
                  </div>
                ))}
              </div>
            ))}
          </div>,
          // Inside an open modal dialog the rest of the page can't be
          // clicked, so the list goes in the dialog.
          place.host,
        )}
    </>
  );
}

/** The next option that can be picked, stepping from `from`; `fallback` when none. */
function firstEnabled<T>(
  options: readonly Option<T>[],
  from: number,
  step: number,
  fallback = -1,
): number {
  for (let index = from; index >= 0 && index < options.length; index += step) {
    if (!options[index]?.disabled) return index;
  }
  return fallback;
}

/**
 * A bare dropdown. Give it either `id` (for a `<label htmlFor>` elsewhere) or
 * `label`, which screen readers read out when there is no visible label.
 */
export function Select<T>({
  large = false,
  wide = false,
  ...props
}: SelectProps<T> & {
  id?: string;
  label?: string;
  /** The taller size used in filter bars and forms. */
  large?: boolean;
  /** Fill the width of its container, for dropdowns in table cells. */
  wide?: boolean;
}) {
  return (
    <Dropdown
      {...props}
      className={cx(styles.select, large && styles.large, wide && styles.wide)}
    />
  );
}

interface Entry {
  label: string;
  note?: string;
  index: number;
  disabled: boolean;
}

/** Runs of options that share a group, in their original order. */
function groupsOf<T>(options: readonly Option<T>[]): { group?: string; entries: Entry[] }[] {
  const runs: { group?: string; entries: Entry[] }[] = [];
  options.forEach((option, index) => {
    const last = runs.at(-1);
    const entry = {
      label: option.label,
      note: option.note,
      index,
      disabled: option.disabled ?? false,
    };
    if (last && last.group === option.group) last.entries.push(entry);
    else runs.push({ group: option.group, entries: [entry] });
  });
  return runs;
}

/** A dropdown with its label above it, for filter bars and forms. */
export function SelectField<T>({ label, ...props }: SelectProps<T> & { label: string }) {
  const id = useId();

  return (
    <div className={styles.field}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <Select id={id} large {...props} />
    </div>
  );
}

/**
 * A compact dropdown for toolbars: a pill that reads "Status  All", with the
 * label in grey and the choice in bold.
 */
export function PillSelect<T>({
  label,
  value,
  options,
  onChange,
  isEqual = Object.is,
  display,
  hideLabel = false,
  icon,
}: SelectProps<T> & {
  label: string;
  /** A small logo before the choice, such as the provider of the chosen resource. */
  icon?: ReactNode;
  /** Show only the choice, for pills whose card already says what they are. */
  hideLabel?: boolean;
  /** Shorter text for the pill than the option's label, such as "All". */
  display?: (option: Option<T>) => string;
}) {
  const current = options.find((option) => isEqual(option.value, value));

  return (
    <Dropdown
      label={label}
      value={value}
      options={options}
      onChange={onChange}
      isEqual={isEqual}
      className={styles.pill}
    >
      {!hideLabel && <span className={styles.pillLabel}>{label}</span>}
      {icon}
      <span className={styles.pillValue}>
        {current ? (display?.(current) ?? current.label) : ''}
      </span>
      <ChevronDown size={13} aria-hidden="true" className={styles.pillChevron} />
    </Dropdown>
  );
}

/**
 * A dropdown that picks several: each option has a tick, the list stays
 * open while you pick, and the closed dropdown names what's picked, or
 * `allLabel` when everything is.
 */
export function MultiSelectField<T>({
  label,
  values,
  options,
  onChange,
  allLabel,
  noneLabel = 'None',
  isEqual = Object.is,
}: {
  label: string;
  values: readonly T[];
  options: readonly Option<T>[];
  onChange: (values: T[]) => void;
  allLabel: string;
  noneLabel?: string;
  isEqual?: (a: T, b: T) => boolean;
}) {
  const id = useId();
  const listId = useId();
  const [open, setOpen] = useState(false);
  const [place, setPlace] = useState<{
    left: number;
    top: number;
    minWidth: number;
    room: number;
    host: Element;
  }>();
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const has = (value: T) => values.some((item) => isEqual(item, value));
  const picked = options.filter((option) => has(option.value));
  const shown =
    picked.length === 0
      ? noneLabel
      : picked.length === options.length
        ? allLabel
        : picked.map((option) => option.label).join(', ');

  useEffect(() => {
    if (!open) return;
    const away = (event: Event) => {
      const target = event.target as Node;
      if (list.current?.contains(target) || button.current?.contains(target)) return;
      setOpen(false);
    };
    const shut = () => {
      setOpen(false);
    };
    // Escape closes the list, not the dialog around it.
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      button.current?.focus();
    };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', escape, true);
    window.addEventListener('resize', shut);
    return () => {
      document.removeEventListener('mousedown', away);
      document.removeEventListener('keydown', escape, true);
      window.removeEventListener('resize', shut);
    };
  }, [open]);

  useEffect(() => {
    const element = list.current;
    if (!open || !element || typeof element.showPopover !== 'function') return;
    element.showPopover();
    return () => {
      if (element.isConnected) element.hidePopover();
    };
  }, [open, place]);

  const toggle = (value: T) => {
    onChange(
      has(value)
        ? values.filter((item) => !isEqual(item, value))
        : options
            .filter((option) => has(option.value) || isEqual(option.value, value))
            .map((option) => option.value),
    );
  };

  return (
    <div className={styles.field}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <button
        ref={button}
        id={id}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        className={cx(styles.select, styles.large)}
        onClick={() => {
          if (open) {
            setOpen(false);
            return;
          }
          const box = button.current?.getBoundingClientRect();
          if (!box) return;
          setPlace({
            left: Math.min(box.left, window.innerWidth - LIST_WIDTH),
            top: box.bottom + GAP,
            minWidth: box.width,
            room: window.innerHeight - box.bottom - EDGE,
            host: button.current?.closest('dialog') ?? document.body,
          });
          setOpen(true);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && open) {
            event.preventDefault();
            setOpen(false);
          }
        }}
      >
        <span className={styles.value}>{shown}</span>
        <ChevronDown size={13} aria-hidden="true" className={styles.chevron} />
      </button>
      {open &&
        place &&
        createPortal(
          <div
            ref={list}
            id={listId}
            role="listbox"
            aria-multiselectable="true"
            popover="manual"
            aria-label={label}
            className={styles.menu}
            style={{
              left: place.left,
              top: place.top,
              minWidth: place.minWidth,
              maxHeight: Math.min(MAX_LIST_HEIGHT, place.room),
            }}
          >
            {options.map((option, index) => {
              const on = has(option.value);
              return (
                <div
                  key={`${option.label}-${String(index)}`}
                  role="option"
                  aria-selected={on}
                  tabIndex={0}
                  className={styles.menuItem}
                  onMouseDown={(event) => {
                    event.preventDefault();
                  }}
                  onClick={() => {
                    toggle(option.value);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault();
                      toggle(option.value);
                    } else if (event.key === 'Escape') {
                      setOpen(false);
                      button.current?.focus();
                    }
                  }}
                >
                  <span>
                    {option.label}
                    {option.note && <span className={styles.note}> · {option.note}</span>}
                  </span>
                  {on && <Check size={14} aria-hidden="true" className={styles.menuCheck} />}
                </div>
              );
            })}
          </div>,
          place.host,
        )}
    </div>
  );
}
