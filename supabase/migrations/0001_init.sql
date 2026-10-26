-- ACL 公会账号系统 / Supabase 初始化
-- 执行方式：Supabase Dashboard → SQL Editor 全选执行，或通过 Supabase MCP 执行
-- 设计见 design/guild-account-design.md

-- ============ 表结构 ============

create table if not exists public.profiles (
	id uuid primary key references auth.users (id) on delete cascade,
	display_name text not null,
	avatar_url text,
	role text not null default 'member' check (role in ('member', 'officer', 'admin')),
	created_at timestamptz not null default now()
);

create table if not exists public.announcements (
	id uuid primary key default gen_random_uuid(),
	title text not null,
	body text not null,
	pinned boolean not null default false,
	author_id uuid references public.profiles (id) on delete set null,
	published_at timestamptz not null default now()
);

create table if not exists public.guild_packs (
	id uuid primary key default gen_random_uuid(),
	name text not null,
	slug text unique,
	mc_version text not null,
	loader text,
	description text,
	url text not null,
	sha256 text,
	size_bytes bigint,
	created_at timestamptz not null default now()
);

create table if not exists public.guild_servers (
	id uuid primary key default gen_random_uuid(),
	name text not null,
	host text not null,
	port integer not null default 25565 check (port between 1 and 65535),
	description text,
	whitelist_required boolean not null default false,
	created_at timestamptz not null default now()
);

create table if not exists public.presence (
	user_id uuid primary key references public.profiles (id) on delete cascade,
	instance_name text,
	mc_version text,
	loader text,
	visible boolean not null default true,
	last_seen timestamptz not null default now()
);

-- ============ 角色判定辅助函数 ============
-- 注意：不能命名为 current_role，那是 PostgreSQL 内置函数名

create or replace function public.acl_role()
	returns text
	language sql
	stable
	security definer
	set search_path = public
as $$
	select coalesce((select role from public.profiles where id = auth.uid()), 'anon');
$$;

create or replace function public.acl_is_member()
	returns boolean
	language sql
	stable
as $$
	select public.acl_role() in ('member', 'officer', 'admin');
$$;

create or replace function public.acl_is_officer()
	returns boolean
	language sql
	stable
as $$
	select public.acl_role() in ('officer', 'admin');
$$;

-- 防止用户自行提权：非 admin 不得修改自己的 role
create or replace function public.guard_profile_role()
	returns trigger
	language plpgsql
	security definer
	set search_path = public
as $$
begin
	if new.role is distinct from old.role and public.acl_role() <> 'admin' then
		raise exception '只有 admin 可以修改角色';
	end if;
	return new;
end;
$$;

drop trigger if exists guard_profile_role on public.profiles;
create trigger guard_profile_role
	before update on public.profiles
	for each row execute function public.guard_profile_role();

-- ============ 行级安全 ============

alter table public.profiles enable row level security;
alter table public.announcements enable row level security;
alter table public.guild_packs enable row level security;
alter table public.guild_servers enable row level security;
alter table public.presence enable row level security;

-- 项目创建时关闭了「自动暴露新表」，需显式授权
grant usage on schema public to authenticated;
grant select, insert, update on public.profiles to authenticated;
grant select on public.announcements, public.guild_packs, public.guild_servers to authenticated;
grant insert, update, delete on public.announcements, public.guild_packs, public.guild_servers to authenticated;
grant select, insert, update, delete on public.presence to authenticated;

-- profiles：成员互读；本人可建（只能是 member）；本人可改自己的资料
drop policy if exists profiles_select on public.profiles;
create policy profiles_select on public.profiles
	for select to authenticated
	using (public.acl_is_member() or id = auth.uid());

drop policy if exists profiles_insert_self on public.profiles;
create policy profiles_insert_self on public.profiles
	for insert to authenticated
	with check (id = auth.uid() and role = 'member');

drop policy if exists profiles_update_self on public.profiles;
create policy profiles_update_self on public.profiles
	for update to authenticated
	using (id = auth.uid() or public.acl_role() = 'admin')
	with check (id = auth.uid() or public.acl_role() = 'admin');

-- 公告 / 整合包 / 服务器：成员可读，officer 以上可写
drop policy if exists announcements_read on public.announcements;
create policy announcements_read on public.announcements
	for select to authenticated
	using (public.acl_is_member());

drop policy if exists announcements_write on public.announcements;
create policy announcements_write on public.announcements
	for all to authenticated
	using (public.acl_is_officer())
	with check (public.acl_is_officer());

drop policy if exists guild_packs_read on public.guild_packs;
create policy guild_packs_read on public.guild_packs
	for select to authenticated
	using (public.acl_is_member());

drop policy if exists guild_packs_write on public.guild_packs;
create policy guild_packs_write on public.guild_packs
	for all to authenticated
	using (public.acl_is_officer())
	with check (public.acl_is_officer());

drop policy if exists guild_servers_read on public.guild_servers;
create policy guild_servers_read on public.guild_servers
	for select to authenticated
	using (public.acl_is_member());

drop policy if exists guild_servers_write on public.guild_servers;
create policy guild_servers_write on public.guild_servers
	for all to authenticated
	using (public.acl_is_officer())
	with check (public.acl_is_officer());

-- 在线状态：本人可写；成员可读可见者，隐身时只有本人可见
drop policy if exists presence_read on public.presence;
create policy presence_read on public.presence
	for select to authenticated
	using (public.acl_is_member() and (visible or user_id = auth.uid()));

drop policy if exists presence_write on public.presence;
create policy presence_write on public.presence
	for insert to authenticated
	with check (user_id = auth.uid());

drop policy if exists presence_update on public.presence;
create policy presence_update on public.presence
	for update to authenticated
	using (user_id = auth.uid())
	with check (user_id = auth.uid());

drop policy if exists presence_delete on public.presence;
create policy presence_delete on public.presence
	for delete to authenticated
	using (user_id = auth.uid());

-- ============ 在线状态查询视图 ============
-- 只返回 3 分钟内有心跳的记录

create or replace view public.online_members
	with (security_invoker = true)
as
select
	p.user_id,
	pr.display_name,
	pr.avatar_url,
	p.instance_name,
	p.mc_version,
	p.loader,
	p.last_seen
from public.presence p
	join public.profiles pr on pr.id = p.user_id
where p.visible
	and p.last_seen > now() - interval '3 minutes';

grant select on public.online_members to authenticated;
