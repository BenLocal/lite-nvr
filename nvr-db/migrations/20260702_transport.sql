-- Record-segment transport (搬运) targets and per-segment upload jobs.
-- A target is a remote storage destination (FTP / SMB / later S3). `config`
-- holds the kind-specific settings as JSON so new backends need no schema change.
CREATE TABLE IF NOT EXISTS "transport_targets" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL DEFAULT '',
    "kind" TEXT NOT NULL DEFAULT '',
    "enabled" INTEGER NOT NULL DEFAULT 1,
    "config" TEXT NOT NULL DEFAULT '{}',
    "remark" TEXT NOT NULL DEFAULT '',
    "create_time" TEXT NOT NULL DEFAULT (datetime('now')),
    "update_time" TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY("id")
);

-- One row per (segment, target): the transport worker copies each recorded
-- segment to every enabled target and records the outcome here so it can retry
-- failures and skip already-uploaded segments. Local files are kept (copy, not
-- move), so this table is purely upload bookkeeping.
CREATE TABLE IF NOT EXISTS "transport_jobs" (
    "id" TEXT NOT NULL,
    "segment_id" TEXT NOT NULL,
    "target_id" TEXT NOT NULL,
    "status" INTEGER NOT NULL DEFAULT 0,
    "attempts" INTEGER NOT NULL DEFAULT 0,
    "remote_key" TEXT NOT NULL DEFAULT '',
    "file_size" INTEGER NOT NULL DEFAULT 0,
    "error" TEXT NOT NULL DEFAULT '',
    "create_time" TEXT NOT NULL DEFAULT (datetime('now')),
    "update_time" TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY("id")
);

CREATE UNIQUE INDEX IF NOT EXISTS "transport_jobs_seg_target_uk" ON "transport_jobs" ("segment_id", "target_id");
CREATE INDEX IF NOT EXISTS "transport_jobs_status_idx" ON "transport_jobs" ("target_id", "status");
