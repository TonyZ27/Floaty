-- Collapse features: collapsed state (0 or 1)
ALTER TABLE stickies ADD COLUMN collapsed INTEGER NOT NULL DEFAULT 0;
