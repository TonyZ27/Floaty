export interface Sticky {
  id: string;
  title: string;
  x: number | null;
  y: number | null;
  w: number;
  h: number;
  pinned: number;
  bg_color: string;
  opacity: number;
  font_size: number;
  font_color: string | null;
  z_order: number;
  hidden: number;
  collapsed: number;
  collapsed_w: number;
  created_at: number;
  updated_at: number;
  deleted_at: number | null;
}

export interface Item {
  id: string;
  sticky_id: string;
  content_md: string;
  due_at: number | null;
  completed_at: number | null;
  sort_order: number;
  created_at: number;
  updated_at: number;
}

export interface ItemUpsert {
  id: string | null;
  sticky_id: string;
  content_md: string;
  due_at: number | null;
  sort_order: number;
}

export interface StickyPatch {
  title?: string;
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  pinned?: number;
  bg_color?: string;
  opacity?: number;
  font_size?: number;
  font_color?: string;
  z_order?: number;
  hidden?: number;
  collapsed?: number;
  collapsed_w?: number;
}
