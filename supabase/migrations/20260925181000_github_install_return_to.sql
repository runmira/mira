-- Where to send the user back to after installing (their local Mira).
alter table public.github_install_states add column return_to text not null default '';
