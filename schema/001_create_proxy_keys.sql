CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS proxy_keys (
  id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id        TEXT NOT NULL DEFAULT 'default',
  user_id          TEXT NOT NULL,
  name             TEXT,
  token_prefix     TEXT NOT NULL,
  hash_algo        TEXT NOT NULL DEFAULT 'argon2id',
  hash_params      JSONB NOT NULL DEFAULT '{}'::jsonb,
  key_hash         TEXT NOT NULL,
  scopes           TEXT[] NOT NULL DEFAULT '{}',
  created_by       TEXT,
  creator_email    TEXT,
  creator_groups   TEXT[] NOT NULL DEFAULT '{}',
  created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_used_at     TIMESTAMPTZ,
  last_used_ip     INET,
  expires_at       TIMESTAMPTZ,
  revoked_at       TIMESTAMPTZ,
  CONSTRAINT ux_proxy_keys_tenant_prefix UNIQUE (tenant_id, token_prefix),
  CONSTRAINT ck_time_exp CHECK (expires_at IS NULL OR expires_at > created_at),
  CONSTRAINT ck_time_rev CHECK (revoked_at IS NULL OR revoked_at > created_at),
  CONSTRAINT ck_prefix_len CHECK (length(token_prefix)=24)
);

-- Ensure quick lookup by prefix (used for auth) and uniqueness to prevent duplicates.
CREATE UNIQUE INDEX IF NOT EXISTS idx_proxy_keys_token_prefix ON proxy_keys (token_prefix);
CREATE INDEX IF NOT EXISTS idx_proxy_keys_tenant_user_created ON proxy_keys (tenant_id, user_id, created_at DESC);

-- Argon2id parameters currently: m=65536 (64MiB), t=3, p=1 (see pat.rs argon2_instance())

CREATE INDEX IF NOT EXISTS ix_proxy_keys_user ON proxy_keys(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS ix_proxy_keys_active ON proxy_keys(tenant_id) WHERE revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS ix_proxy_keys_last_used ON proxy_keys(last_used_at);
CREATE INDEX IF NOT EXISTS idx_proxy_keys_creator_email_active ON proxy_keys(creator_email) WHERE revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS ix_proxy_keys_revoked_at ON proxy_keys(revoked_at) WHERE revoked_at IS NOT NULL;
