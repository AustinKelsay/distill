-- Distill Library schema v6: current-session list paging index for scale budgets.
-- Keep this expression byte-identical to the list_sessions ORDER BY/cursor key.
CREATE INDEX idx_sessions_list_page
  ON sessions (COALESCE(updated_at, ''), id)
  WHERE successful_projection_generation > 0;
