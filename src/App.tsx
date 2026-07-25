import { getCurrentWindow } from "@tauri-apps/api/window";
import { StickyPage } from "./sticky/StickyPage";
import { PreferencesPage } from "./preferences/PreferencesPage";
import { SearchPage } from "./search/SearchPage";
import { WindowPickerPage } from "./assignment/WindowPickerPage";

export type Route =
  | { kind: "sticky"; stickyId: string }
  | { kind: "window-picker"; stickyId: string }
  | { kind: "preferences" }
  | { kind: "search" }
  | { kind: "unknown" };

const CANONICAL_ULID = /^[0-7][0-9A-HJKMNP-TV-Z]{25}$/;

export function parseWindowLabel(label: string): Route {
  const pickerPrefix = "window-picker-";
  if (label.startsWith(pickerPrefix)) {
    const stickyId = label.slice(pickerPrefix.length);
    return CANONICAL_ULID.test(stickyId)
      ? { kind: "window-picker", stickyId }
      : { kind: "unknown" };
  }
  if (label.startsWith("sticky-")) {
    const stickyId = label.slice("sticky-".length);
    return CANONICAL_ULID.test(stickyId)
      ? { kind: "sticky", stickyId }
      : { kind: "unknown" };
  }
  if (label === "preferences") {
    return { kind: "preferences" };
  }
  if (label === "search") {
    return { kind: "search" };
  }
  return { kind: "unknown" };
}

export default function App() {
  const route = parseWindowLabel(getCurrentWindow().label);
  if (route.kind === "preferences") {
    return <PreferencesPage />;
  }
  if (route.kind === "search") {
    return <SearchPage />;
  }
  if (route.kind === "sticky") {
    return <StickyPage stickyId={route.stickyId} />;
  }
  if (route.kind === "window-picker") {
    return <WindowPickerPage stickyId={route.stickyId} />;
  }
  return (
    <div className="h-screen bg-red-100 p-3 text-xs">
      Unknown window label.
    </div>
  );
}
