/**
 * Things the app asks of the computer rather than the engine: opening a link
 * in the browser, showing the folder picker and quitting.
 */
import { getBridge } from './bridge';

/** Opens an http(s) link in the user's browser. */
export function openExternal(url: string): Promise<void> {
  return getBridge().openExternal(url);
}

/** Shows the file picker for a `.hoista` transfer file. Resolves to its path, or null. */
export function chooseTransferFile(): Promise<string | null> {
  return getBridge().chooseTransferFile();
}

/** Where a file dropped on the window lives on disk, or null when it has no path. */
export function pathForFile(file: File): string | null {
  return getBridge().pathForFile(file);
}

/** Shows the save dialog for a new `.hoista` transfer file. Resolves to its path, or null. */
export function chooseTransferSave(): Promise<string | null> {
  return getBridge().chooseTransferSave();
}

/** Quits Hoista. */
export function quitProject(): Promise<void> {
  return getBridge().quit();
}
