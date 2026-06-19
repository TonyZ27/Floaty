-- Collapse features: collapsed width
ALTER TABLE stickies ADD COLUMN collapsed_w INTEGER NOT NULL DEFAULT 200;
