-- Dedicated sqlator_* names. Applied in-test against a file DB.

CREATE TABLE IF NOT EXISTS sqlator_persons (
  person_id INTEGER PRIMARY KEY,
  last_name TEXT NOT NULL,
  first_name TEXT
);

CREATE TABLE IF NOT EXISTS sqlator_orders (
  order_id INTEGER PRIMARY KEY,
  order_number INTEGER NOT NULL,
  person_id INTEGER,
  total_amount REAL,
  FOREIGN KEY (person_id) REFERENCES sqlator_persons (person_id)
);

CREATE TABLE IF NOT EXISTS sqlator_paged_items (
  id INTEGER PRIMARY KEY,
  n INTEGER NOT NULL
);

INSERT OR IGNORE INTO sqlator_persons (person_id, last_name, first_name)
VALUES (1, 'Smith', 'John');

INSERT OR IGNORE INTO sqlator_persons (person_id, last_name, first_name)
VALUES (2, 'Garcia', 'Maria');

INSERT OR IGNORE INTO sqlator_orders (order_id, order_number, person_id, total_amount)
VALUES (101, 55001, 1, 150.75);

INSERT OR IGNORE INTO sqlator_paged_items (id, n)
SELECT seq, 1251 - seq FROM (
  WITH RECURSIVE seq (seq) AS (
    SELECT 1
    UNION ALL
    SELECT seq + 1 FROM seq WHERE seq < 1250
  )
  SELECT seq FROM seq
);
