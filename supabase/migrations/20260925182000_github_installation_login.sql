-- The linked user's GitHub login, so tokens are only handed out for
-- repositories they administer (org members may only have read access).
alter table public.github_installations add column github_login text not null default '';
