ALTER TABLE app_domains ADD COLUMN job TEXT NOT NULL DEFAULT 'serve' CHECK (job IN ('serve', 'redirect'));
ALTER TABLE app_domains ADD COLUMN target TEXT;
ALTER TABLE app_domains ADD COLUMN primary_domain INTEGER NOT NULL DEFAULT 0;
ALTER TABLE app_domains ADD COLUMN dns_provider_id TEXT;

UPDATE app_domains SET job = 'serve', target = 'web', primary_domain = 1 WHERE position = 0;
UPDATE app_domains
SET job = 'redirect',
    target = (SELECT p.domain FROM app_domains p WHERE p.app_id = app_domains.app_id AND p.position = 0)
WHERE position > 0;
