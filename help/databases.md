# Databases and roles

Ferrum runs one PostgreSQL server on the box, listening on loopback only. Each database has an owner role, and can have restricted roles under it.

## The owner

Created with the database. It owns every table, runs your migrations, and is what `DATABASE_URL` points at unless your file says otherwise.

Row-level security never applies to the owner of a table, unless your migrations say `FORCE ROW LEVEL SECURITY` on it. Then the owner needs the `BYPASSRLS` attribute to see every row, and the file asks for it:

```toml
[database]
url = "DATABASE_ADMIN_URL"
bypass_rls = true
```

Ferrum sets it on the owner at link and on every deploy that follows the file, and takes it away again when the line goes. Restricted roles never get it.

## Restricted roles

A login with a password and a connection limit that can connect, and nothing else. What it may read or write is decided by your own migrations (`GRANT`, row-level security policies). It never gets superuser or bypass-RLS powers. This is how a multi-tenant app serves each tenant as a user the database rules apply to, while migrations run as the owner.

Roles are declared in `ferrum.toml` and created when the app is linked to a database or deployed from a tag that adds one, or added by hand on the Databases page:

```toml
[database]
url = "DATABASE_ADMIN_URL"

[database.roles.app]
url = "DATABASE_URL"
```

A role dropped from the file is kept; the app's page tells you so.

## Labels

The label is the environment name your code reads an address under. The file names one for the owner, one per role, and one for Redis:

```toml
[redis]
url = "CACHE_URL"
```

With no file the labels are editable next to the database link on the app's page. A second linked database is `<NAME>_DATABASE_URL` by default. Ferrum rewrites every address on each rotate, so a label never points at a stale password.

## Rotate, restore, delete

Rotate is per role. A restore loads the dump as the owner and re-grants the database's roles their access. Deleting the database drops every role with it.

## Connecting from your machine

See *Connect from your machine*: the Databases page shows the SSH tunnel command and a copy-URL control per role.
