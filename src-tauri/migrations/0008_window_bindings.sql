CREATE TABLE window_bindings (
  sticky_id TEXT PRIMARY KEY REFERENCES stickies(id) ON DELETE CASCADE,
  app_bundle_id TEXT NOT NULL,
  app_name TEXT NOT NULL,
  window_title TEXT NOT NULL DEFAULT '',
  runtime_pid INTEGER,
  runtime_window_id INTEGER,
  state TEXT NOT NULL CHECK (
    state IN ('active', 'target_closed', 'reconnect_required', 'unavailable')
  ),
  prior_pinned INTEGER NOT NULL CHECK (prior_pinned IN (0, 1)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX idx_window_bindings_state ON window_bindings(state);
