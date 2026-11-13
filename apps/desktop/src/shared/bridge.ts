/**
 * `window.hoista`: what the screens can ask of the app.
 *
 * This is the only way the UI reaches the engine. It carries local API
 * requests and events, opens links and the system file pickers, moves
 * between the main window and the menu bar panel, and quits the app.
 * Nothing else: no Node, no file system access, no tokens.
 */
import type { EngineEvent, EngineRequest, EngineResult } from './api';

/** Operating systems the app lays itself out for. */
export type Platform = 'darwin' | 'linux' | 'win32';

/** `window.hoista` */
export interface HoistaBridge {
  /** For platform-specific layout, such as room for the macOS window buttons. */
  platform: Platform;
  request(request: EngineRequest): Promise<EngineResult>;
  /** Calls `listener` for every engine event. Returns a function that stops it. */
  onEvent(listener: (event: EngineEvent) => void): () => void;
  /** Opens an http(s) link in the user's browser. */
  openExternal(url: string): Promise<void>;
  /** Shows the system folder picker. Resolves to the chosen path, or null if cancelled. */
  chooseFolder(): Promise<string | null>;
  /** Shows the file picker for a `.hoista` file from another computer, or null if cancelled. */
  chooseTransferFile(): Promise<string | null>;
  /** Where a file dropped on the window lives on disk, or null when it has no path. */
  pathForFile(file: File): string | null;
  /** Shows the save dialog for a new `.hoista` file. Resolves to the chosen path, or null if cancelled. */
  chooseTransferSave(): Promise<string | null>;
  /** Quits Hoista. Chat apps can't reach it again until it is reopened. */
  quit(): Promise<void>;
  /** Brings the main window forward on a screen: a route such as `/projects/moodboard`, or `settings`. */
  showMain(target?: string): Promise<void>;
  /** Main window only: called when the menu bar or a notification asks for a screen. */
  onNavigate(listener: (target: string) => void): () => void;
  /** Menu bar panel only: hides the panel. */
  closePanel(): Promise<void>;
  /** Menu bar panel only: fits the panel to its content's height. */
  resizePanel(height: number): Promise<void>;
  /**
   * Reloads every Hoista window, after switching to another Hoista, so the
   * panel shows the same one as the main window.
   */
  reloadWindows(): Promise<void>;
}
