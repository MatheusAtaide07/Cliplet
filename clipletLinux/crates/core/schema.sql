-- schema.sql — opened by the capture daemon and by the Tauri app
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;    -- per connection: both processes must enable it
PRAGMA busy_timeout = 5000;  -- per connection

CREATE TABLE devices (
  id          TEXT PRIMARY KEY,                 -- UUID, same as in the cloud
  name        TEXT NOT NULL,
  platform    TEXT NOT NULL CHECK (platform IN ('ios','ipados','macos','linux')),
  is_current  INTEGER NOT NULL DEFAULT 0 CHECK (is_current IN (0,1)),
  revoked_at  INTEGER,                          -- epoch ms
  remote_seq  INTEGER                           -- last server_seq seen
) WITHOUT ROWID;
CREATE UNIQUE INDEX devices_one_current ON devices(is_current) WHERE is_current = 1;

CREATE TABLE entries (
  seq                  INTEGER PRIMARY KEY,     -- stable rowid, used by FTS
  id                   TEXT NOT NULL UNIQUE,    -- UUIDv7, same as in the cloud
  device_id            TEXT NOT NULL REFERENCES devices(id),
  kind                 TEXT NOT NULL
                         CHECK (kind IN ('text','image','color','file','link','unknown')),
  source               TEXT NOT NULL DEFAULT 'clipboard'
                         CHECK (source IN ('clipboard','share_sheet','keyboard','manual')),
  created_at           INTEGER NOT NULL,        -- epoch ms, first copy (never changes)
  last_used_at         INTEGER NOT NULL,        -- epoch ms, last copy, paste or open
  remote_last_used_at  INTEGER,                 -- last_used_at known to the cloud
  content_hash         TEXT NOT NULL,           -- HMAC-SHA256 (hex), key derived from the space key
  preview_text         TEXT,                    -- ~200 chars, color hex, URL or file name
  thumbnail            BLOB,                    -- small WebP
  text_content         TEXT,                    -- small content (up to 64 KB)
  file_path            TEXT,                    -- relative to $XDG_DATA_HOME/<app id>/
  file_name            TEXT,
  mime_type            TEXT,
  byte_size            INTEGER NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  sync_status          TEXT NOT NULL DEFAULT 'local_only' CHECK (sync_status IN
                         ('local_only','pending_upload','synced',
                          'remote_only','pending_download','pending_remote_delete')),
  dirty                INTEGER NOT NULL DEFAULT 0 CHECK (dirty IN (0,1)), -- boards to upload
  remote_seq           INTEGER,
  sync_error           TEXT,
  CHECK (text_content IS NULL OR file_path IS NULL),
  CHECK (sync_status NOT IN ('local_only','pending_upload','synced')
         OR text_content IS NOT NULL OR file_path IS NOT NULL)
);
CREATE INDEX entries_recent        ON entries(last_used_at DESC);
CREATE INDEX entries_kind_recent   ON entries(kind, last_used_at DESC);
CREATE INDEX entries_device_recent ON entries(device_id, last_used_at DESC);
CREATE INDEX entries_hash          ON entries(content_hash);
CREATE INDEX entries_sync_queue    ON entries(sync_status)
  WHERE sync_status IN ('pending_upload','pending_download','pending_remote_delete') OR dirty = 1;

CREATE TABLE boards (
  id           TEXT PRIMARY KEY,                -- UUID, same as in the cloud
  name         TEXT NOT NULL,
  color        TEXT NOT NULL CHECK (color GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
  position     REAL NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  sync_status  TEXT NOT NULL DEFAULT 'local_only'
                 CHECK (sync_status IN ('local_only','pending_upload','synced','pending_remote_delete')),
  dirty        INTEGER NOT NULL DEFAULT 0 CHECK (dirty IN (0,1)),
  remote_seq   INTEGER
) WITHOUT ROWID;

CREATE TABLE entry_boards (
  entry_id  TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  board_id  TEXT NOT NULL REFERENCES boards(id)  ON DELETE CASCADE,
  added_at  INTEGER NOT NULL,
  PRIMARY KEY (entry_id, board_id)
) WITHOUT ROWID;
CREATE INDEX entry_boards_board ON entry_boards(board_id, added_at DESC);

-- Text search (external content: the text lives only in entries)
CREATE VIRTUAL TABLE entries_fts USING fts5(
  preview_text, text_content, file_name,
  content = 'entries', content_rowid = 'seq',
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER entries_fts_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts(rowid, preview_text, text_content, file_name)
  VALUES (new.seq, new.preview_text, new.text_content, new.file_name);
END;
CREATE TRIGGER entries_fts_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, preview_text, text_content, file_name)
  VALUES ('delete', old.seq, old.preview_text, old.text_content, old.file_name);
END;
CREATE TRIGGER entries_fts_au AFTER UPDATE OF preview_text, text_content, file_name ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, preview_text, text_content, file_name)
  VALUES ('delete', old.seq, old.preview_text, old.text_content, old.file_name);
  INSERT INTO entries_fts(rowid, preview_text, text_content, file_name)
  VALUES (new.seq, new.preview_text, new.text_content, new.file_name);
END;

-- Boards of entries going to the cloud go too
CREATE TRIGGER entries_upload_queues_boards
AFTER UPDATE OF sync_status ON entries
WHEN NEW.sync_status = 'pending_upload'
BEGIN
  UPDATE boards SET sync_status = 'pending_upload'
   WHERE sync_status = 'local_only'
     AND id IN (SELECT board_id FROM entry_boards WHERE entry_id = NEW.id);
END;

-- A board change on an entry that exists in the cloud must be uploaded.
-- When the worker applies received boards (only if dirty = 0), it resets dirty in the same transaction.
CREATE TRIGGER entry_boards_ai AFTER INSERT ON entry_boards BEGIN
  UPDATE entries SET dirty = 1
   WHERE id = NEW.entry_id AND sync_status IN ('synced','remote_only','pending_download');
  UPDATE boards SET sync_status = 'pending_upload'
   WHERE id = NEW.board_id AND sync_status = 'local_only'
     AND EXISTS (SELECT 1 FROM entries WHERE id = NEW.entry_id AND sync_status <> 'local_only');
END;
CREATE TRIGGER entry_boards_ad AFTER DELETE ON entry_boards BEGIN
  UPDATE entries SET dirty = 1
   WHERE id = OLD.entry_id AND sync_status IN ('synced','remote_only','pending_download');
END;
CREATE TRIGGER boards_edit AFTER UPDATE OF name, color, position ON boards
WHEN OLD.sync_status = 'synced'
BEGIN
  UPDATE boards SET dirty = 1 WHERE id = NEW.id;
END;

CREATE TABLE settings (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL
) WITHOUT ROWID;
INSERT INTO settings(key, value) VALUES
  ('auto_upload',        '0'),      -- feature 7: nothing is uploaded unless the user asks
  ('retention_days',     '30'),     -- feature 6: 0 = never delete
  ('keep_board_entries', '1'),      -- entries in boards never expire
  ('inline_max_bytes',   '65536');  -- above this, content goes to a file

CREATE TABLE sync_state (
  key    TEXT PRIMARY KEY,          -- 'cursor:devices', 'cursor:boards', 'cursor:entries', 'last_pull_at'
  value  TEXT NOT NULL
) WITHOUT ROWID;
