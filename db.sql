create table if not exists crates
(
  id serial not null
    constraint crates_pk
      primary key,
  name varchar(64) not null
);

comment on column crates.name is 'crate names are limited to 64 characters, see https://github.com/rust-lang/crates.io/pull/718';

create unique index if not exists crates_name_uindex
  on crates (name);

create table if not exists subscriptions
(
  user_id bigint not null,
  crate_id int not null,
  constraint subscriptions_pk
    primary key (crate_id, user_id)
);

comment on column subscriptions.user_id is 'telegram user id (yeah, telegram is hardcoded)';

create index if not exists subscriptions_user_id_index
  on subscriptions (user_id)
    include (crate_id);

-- will error if executed twice
alter table subscriptions
  add constraint subscriptions_crates_id_fk
    foreign key (crate_id) references crates
      on delete cascade;

create or replace procedure subscribe(_user_id bigint, _crate varchar(64))
    LANGUAGE plpgsql
AS $$
begin
    if not exists (select * from crates where crates.name = _crate) then
        begin
            insert into crates (name) values (_crate);
        exception
            when unique_violation then
        end;
    end if;

    begin
        insert into subscriptions (user_id, crate_id)
            select _user_id, id from crates
                where crates.name = _crate;
    exception
        when unique_violation then
    end;
end
$$;

create or replace procedure unsubscribe(_user_id bigint, _crate varchar(64))
    LANGUAGE plpgsql
AS $$
begin
    delete from subscriptions
        where crate_id = (select id from crates where name = _crate)
            and user_id = _user_id;
end
$$;

create or replace function list_subscriptions(_user_id bigint)
RETURNS TABLE(crate_name varchar(64))
    LANGUAGE plpgsql
AS $$
begin
    RETURN QUERY select c.name as crate_name
        from subscriptions as s
            inner join crates as c on c.id = s.crate_id
        where s.user_id = _user_id;
end
$$;

create or replace function list_subscribers(_crate varchar(64))
    RETURNS TABLE(user_id bigint)
    LANGUAGE plpgsql
AS $$
begin
    RETURN QUERY select s.user_id as user_id
         from subscriptions as s
              inner join crates as c on c.id = s.crate_id
         where c.name = _crate;
end
$$;
