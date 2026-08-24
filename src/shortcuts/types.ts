export interface KeyCombo {
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
  meta?: boolean;
  key?: string; // Standardized key, e.g. "N", "W", "P", "Comma", "Backspace", "Space"
}

export interface ShortcutDef {
  id: string;
  label: string;
  description: string;
  defaultCombo: KeyCombo;
  isGlobal?: boolean;
}

export const DEFAULT_SHORTCUTS: ShortcutDef[] = [
  {
    id: "shortcut_global_new_sticky",
    label: "全局：新建便签",
    description: "在任何 App 下快速新建便签",
    defaultCombo: { meta: true, shift: true, key: "N" },
    isGlobal: true,
  },
  {
    id: "shortcut_sticky_new",
    label: "便签聚焦时：新建便签",
    description: "当前便签聚焦时新建一张便签",
    defaultCombo: { meta: true, key: "N" },
  },
  {
    id: "shortcut_sticky_hide",
    label: "隐藏当前便签",
    description: "隐藏当前便签窗口 (不删除数据)",
    defaultCombo: { meta: true, key: "W" },
  },
  {
    id: "shortcut_sticky_pin",
    label: "切换置顶状态",
    description: "切换当前便签的窗口置顶",
    defaultCombo: { meta: true, shift: true, key: "P" },
  },
  {
    id: "shortcut_sticky_settings",
    label: "打开/关闭外观设置",
    description: "展开或收起便签颜色与字体设置",
    defaultCombo: { meta: true, key: "," },
  },
  {
    id: "shortcut_sticky_delete",
    label: "删除当前便签",
    description: "弹出删除确认并删除便签",
    defaultCombo: { meta: true, key: "Backspace" },
  },
];
