PRAGMA foreign_keys = ON;

CREATE TABLE accounts (
  id TEXT PRIMARY KEY NOT NULL,
  tenant_id TEXT NOT NULL,
  email_normalized TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
  quota_micros INTEGER NOT NULL,
  profile_json TEXT NOT NULL CHECK(json_valid(profile_json)),
  created_at_ms INTEGER NOT NULL,
  tier TEXT NOT NULL CHECK(tier IN ('free', 'pro')),
  deleted_at_ms INTEGER,
  UNIQUE(tenant_id, email_normalized)
);

CREATE TABLE organizations (
  id TEXT PRIMARY KEY NOT NULL,
  tenant_id TEXT NOT NULL,
  owner_account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
  name TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE TABLE organization_members (
  tenant_id TEXT NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('owner', 'admin', 'member')),
  state TEXT NOT NULL CHECK(state IN ('active', 'suspended')),
  created_at_ms INTEGER NOT NULL,
  PRIMARY KEY(organization_id, account_id)
);

CREATE TABLE projects (
  id TEXT PRIMARY KEY NOT NULL,
  tenant_id TEXT NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  owner_account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
  budget_micros INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  settings_json TEXT NOT NULL CHECK(json_valid(settings_json)),
  created_at_ms INTEGER NOT NULL
);
