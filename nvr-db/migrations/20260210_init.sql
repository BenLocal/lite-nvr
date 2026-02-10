CREATE TABLE IF NOT EXISTS "kvs" (
    "id"	INTEGER NOT NULL,
    "module"	VARCHAR NOT NULL,
    "key"	VARCHAR NOT NULL,
    "sub_key"	VARCHAR NOT NULL,
    "value"	TEXT NOT NULL,
    PRIMARY KEY("id" AUTOINCREMENT)
);

CREATE INDEX IF NOT EXISTS "kvs_module_idx" ON "kvs" ("module");
CREATE INDEX IF NOT EXISTS "kvs_module_key_idx" ON "kvs" ("key", "module");
CREATE INDEX IF NOT EXISTS "kvs_module_sub_key_idx" ON "kvs" ("sub_key", "key", "module");