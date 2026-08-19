-- Dedicated sqlator_* names so existing Persons/Orders volumes do not collide.

CREATE TABLE IF NOT EXISTS sqlator_persons (
  person_id integer PRIMARY KEY,
  last_name varchar(255) NOT NULL,
  first_name varchar(255)
);

CREATE TABLE IF NOT EXISTS sqlator_orders (
  order_id integer PRIMARY KEY,
  order_number integer NOT NULL,
  person_id integer REFERENCES sqlator_persons (person_id),
  total_amount numeric(10, 2)
);

CREATE TABLE IF NOT EXISTS sqlator_paged_items (
  id integer PRIMARY KEY,
  n integer NOT NULL
);

INSERT INTO sqlator_persons (person_id, last_name, first_name)
VALUES (1, 'Smith', 'John')
ON CONFLICT (person_id) DO NOTHING;

INSERT INTO sqlator_persons (person_id, last_name, first_name)
VALUES (2, 'Garcia', 'Maria')
ON CONFLICT (person_id) DO NOTHING;

INSERT INTO sqlator_orders (order_id, order_number, person_id, total_amount)
VALUES (101, 55001, 1, 150.75)
ON CONFLICT (order_id) DO NOTHING;

INSERT INTO sqlator_paged_items (id, n)
SELECT g, 1251 - g FROM generate_series(1, 1250) AS g
ON CONFLICT (id) DO NOTHING;
