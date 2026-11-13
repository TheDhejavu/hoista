-- Hoista's local database.

CREATE TABLE profile (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    name       TEXT    NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 80),
    initials   TEXT    NOT NULL CHECK (length(trim(initials)) BETWEEN 1 AND 4),
    created_at TEXT    NOT NULL
) STRICT;

CREATE TABLE events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    at      TEXT    NOT NULL,
    actor   TEXT    NOT NULL,
    project_id  TEXT,
    kind    TEXT    NOT NULL,
    summary TEXT    NOT NULL,
    subject_id TEXT,
    detail  TEXT
) STRICT;

CREATE TABLE accounts (
    id       TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    data     TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id)
) STRICT;

CREATE TABLE projects (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id)
) STRICT;

CREATE TABLE resources (
    id           TEXT PRIMARY KEY,
    project_id       TEXT REFERENCES projects(id) ON DELETE SET NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    provider_ref TEXT NOT NULL,
    data         TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id),
    UNIQUE(account_id, provider_ref)
) STRICT;

CREATE TABLE links (
    id            TEXT PRIMARY KEY,
    from_resource TEXT NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    to_resource   TEXT NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    data          TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id),
    CHECK(from_resource != to_resource)
) STRICT;

CREATE TABLE plans (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id)
) STRICT;

CREATE TABLE deploys (
    id      TEXT    PRIMARY KEY,
    project_id  TEXT    NOT NULL REFERENCES projects(id),
    version INTEGER NOT NULL,
    status  TEXT    NOT NULL CHECK (status IN ('queued','running','live','failed','rolled_back')),
    data    TEXT    NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id),
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0, 1)),
    UNIQUE(project_id, version)
) STRICT;

CREATE TABLE agents (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id)
) STRICT;

CREATE TABLE approvals (
    id         TEXT PRIMARY KEY,
    agent_id   TEXT NOT NULL REFERENCES agents(id),
    decision   TEXT NOT NULL CHECK(decision IN ('pending','allowed','declined','expired','consumed')),
    expires_at TEXT NOT NULL,
    data       TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id)
) STRICT;

CREATE TABLE secret_metadata (
    resource_id TEXT NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    name    TEXT NOT NULL,
    data    TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY(resource_id, name)
) STRICT;

CREATE TABLE usage (
    resource_id TEXT NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    day     TEXT NOT NULL,
    data    TEXT NOT NULL CHECK(json_valid(data)),
    PRIMARY KEY(resource_id, day)
) STRICT;

CREATE TABLE settings (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    data TEXT    NOT NULL CHECK(json_valid(data))
) STRICT;

CREATE TABLE private_link (
    id   INTEGER PRIMARY KEY CHECK(id=1),
    data TEXT    NOT NULL CHECK(json_valid(data))
);

CREATE TABLE oauth_clients (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK(json_valid(data))
);

CREATE TABLE oauth_pending (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK(json_valid(data))
);

CREATE TABLE oauth_grants (
    agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    data     TEXT NOT NULL CHECK(json_valid(data))
);

CREATE TABLE custom_domains (
    id       TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    resource_id  TEXT NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    hostname TEXT NOT NULL UNIQUE,
    data     TEXT NOT NULL CHECK(json_valid(data) AND json_extract(data,'$.id') = id)
) STRICT;

CREATE TABLE deployment_admissions (
    actor       TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 128),
    request_key TEXT NOT NULL CHECK(length(request_key) = 36),
    fingerprint TEXT NOT NULL CHECK(length(fingerprint) = 64),
    deploy_id   TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    PRIMARY KEY(actor, request_key)
) STRICT;

CREATE TABLE environment_admission_keys (
    reference  TEXT    PRIMARY KEY,
    commitment TEXT    CHECK(commitment IS NULL OR length(commitment) = 64),
    active     INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
    retired    INTEGER NOT NULL DEFAULT 0 CHECK(retired IN (0, 1)),
    CHECK(active = 0 OR commitment IS NOT NULL)
) STRICT;

CREATE TABLE transfer_restores (
    id     TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK(status IN ('staging','installed','abandoned')),
    data   TEXT NOT NULL CHECK(json_valid(data))
) STRICT;

CREATE TABLE ignored_resources (
    resource_id    TEXT PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    ignored_at TEXT NOT NULL
) STRICT;

CREATE TABLE map_layouts (
    project_id   TEXT    PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision BETWEEN 0 AND 9007199254740991),
    data     TEXT    NOT NULL CHECK(json_valid(data))
) STRICT;

CREATE TABLE enrollment_requests (
    id TEXT PRIMARY KEY NOT NULL,
    expires_at TEXT NOT NULL,
    data TEXT NOT NULL
);

CREATE TABLE operations (
    id TEXT PRIMARY KEY NOT NULL,
    key TEXT NOT NULL,
    state TEXT NOT NULL,
    started_at TEXT NOT NULL,
    data TEXT NOT NULL
);

CREATE TABLE webhooks (
    id TEXT PRIMARY KEY NOT NULL,
    cursor INTEGER NOT NULL,
    data TEXT NOT NULL
);

CREATE TABLE webhook_deliveries (
    id TEXT PRIMARY KEY NOT NULL,
    webhook_id TEXT NOT NULL,
    event_id INTEGER NOT NULL,
    state TEXT NOT NULL,
    next_at TEXT NOT NULL,
    data TEXT NOT NULL,
    UNIQUE (webhook_id, event_id)
);

CREATE INDEX deploys_queue ON deploys(status, project_id, version);

CREATE INDEX custom_domains_project ON custom_domains(project_id,id);

CREATE UNIQUE INDEX deploy_successful_release_version
    ON deploys(project_id, json_extract(data, '$.release_version'))
    WHERE json_extract(data, '$.release_version') IS NOT NULL;

CREATE UNIQUE INDEX environment_admission_one_active
    ON environment_admission_keys(active)
    WHERE active = 1;

CREATE UNIQUE INDEX transfer_one_staging
    ON transfer_restores(status)
    WHERE status='staging';

CREATE INDEX operations_by_key ON operations(key, state);

CREATE INDEX operations_by_start ON operations(started_at DESC, id DESC);

CREATE INDEX webhook_deliveries_due ON webhook_deliveries(state, next_at);

CREATE INDEX webhook_deliveries_by_webhook ON webhook_deliveries(webhook_id, event_id DESC);

CREATE INDEX resources_project ON resources(project_id);

CREATE TRIGGER deployment_admissions_no_update BEFORE UPDATE ON deployment_admissions
BEGIN
    SELECT RAISE(ABORT, 'Deployment admissions are immutable');
END;

CREATE TRIGGER deployment_admissions_no_delete BEFORE DELETE ON deployment_admissions
BEGIN
    SELECT RAISE(ABORT, 'Deployment admissions are retained');
END;

CREATE TRIGGER environment_admission_active_immutable BEFORE UPDATE ON environment_admission_keys
WHEN OLD.active = 1
BEGIN
    SELECT RAISE(ABORT, 'Active admission key must be preserved');
END;

CREATE TRIGGER environment_admission_keys_retained BEFORE DELETE ON environment_admission_keys
BEGIN
    SELECT RAISE(ABORT, 'Admission key references must be preserved');
END;

CREATE TRIGGER environment_admission_retired_immutable BEFORE UPDATE ON environment_admission_keys
WHEN OLD.retired = 1
BEGIN
    SELECT RAISE(ABORT, 'Retired admission setup must be preserved');
END;

CREATE TRIGGER transfer_restore_no_delete BEFORE DELETE ON transfer_restores
BEGIN
    SELECT RAISE(ABORT,'Transfer recovery records must be retained');
END;

CREATE TRIGGER transfer_restore_identity_immutable BEFORE UPDATE ON transfer_restores
WHEN NEW.id != OLD.id
  OR NEW.data != OLD.data
  OR OLD.status != 'staging'
  OR NEW.status NOT IN ('installed','abandoned')
BEGIN
    SELECT RAISE(ABORT,'Invalid transfer recovery transition');
END;

CREATE TRIGGER ignore_only_unassigned BEFORE INSERT ON ignored_resources
WHEN EXISTS(SELECT 1 FROM resources WHERE id=NEW.resource_id AND project_id IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT,'Only unassigned resources can be ignored');
END;

CREATE TRIGGER no_group_ignored BEFORE UPDATE OF project_id ON resources
WHEN NEW.project_id IS NOT NULL AND EXISTS(SELECT 1 FROM ignored_resources WHERE resource_id=NEW.id)
BEGIN
    SELECT RAISE(ABORT,'Unignore the resource before grouping it');
END;

CREATE TRIGGER events_no_delete BEFORE DELETE ON events
WHEN OLD.at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 days')
BEGIN
    SELECT RAISE(ABORT, 'Activity from the last 30 days is kept');
END;

CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'Events are append-only');
END;

-- Separate spaces for projects, such as default and production. Every
-- project is in one; a new Hoista starts with default.
CREATE TABLE environments (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL CHECK (json_valid(data) AND json_extract(data, '$.id') = id)
) STRICT;

INSERT INTO environments (id, data)
VALUES (
    'env_default',
    json_object('id', 'env_default', 'name', 'default', 'created_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
