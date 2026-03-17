CREATE TABLE IF NOT EXISTS "record_segments" (
    "id" TEXT NOT NULL,
    "record_type" INTEGER NOT NULL DEFAULT 0,
    "start_time" INTEGER NOT NULL DEFAULT 0,
    "duration" REAL NOT NULL DEFAULT 0,
    "file_size" INTEGER NOT NULL DEFAULT 0,
    "file_name" TEXT NOT NULL DEFAULT '',
    "file_path" TEXT NOT NULL,
    "folder" TEXT NOT NULL DEFAULT '',
    "app" TEXT NOT NULL DEFAULT '',
    "stream" TEXT NOT NULL DEFAULT '',
    "vhost" TEXT NOT NULL DEFAULT '',
    "video_codec" TEXT NOT NULL DEFAULT '',
    "video_width" INTEGER NOT NULL DEFAULT 0,
    "video_height" INTEGER NOT NULL DEFAULT 0,
    "video_fps" REAL NOT NULL DEFAULT 0,
    "video_bit_rate" INTEGER NOT NULL DEFAULT 0,
    "audio_codec" TEXT NOT NULL DEFAULT '',
    "audio_sample_rate" INTEGER NOT NULL DEFAULT 0,
    "audio_channels" INTEGER NOT NULL DEFAULT 0,
    "audio_bit_rate" INTEGER NOT NULL DEFAULT 0,
    "reserve_text1" TEXT NOT NULL DEFAULT '',
    "reserve_text2" TEXT NOT NULL DEFAULT '',
    "reserve_text3" TEXT NOT NULL DEFAULT '',
    "reserve_int1" INTEGER NOT NULL DEFAULT 0,
    "reserve_int2" INTEGER NOT NULL DEFAULT 0,
    "create_time" TEXT NOT NULL DEFAULT (datetime('now')),
    "update_time" TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY("id")
);

CREATE UNIQUE INDEX IF NOT EXISTS "record_segments_file_path_uk" ON "record_segments" ("file_path");
CREATE INDEX IF NOT EXISTS "record_segments_stream_start_idx" ON "record_segments" ("app", "stream", "start_time");
CREATE INDEX IF NOT EXISTS "record_segments_create_time_idx" ON "record_segments" ("create_time");
