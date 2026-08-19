-- Dedicated sqlator_* names (MariaDB 11). Same logical schema as MySQL.

CREATE TABLE IF NOT EXISTS sqlator_persons (
  person_id int PRIMARY KEY,
  last_name varchar(255) NOT NULL,
  first_name varchar(255)
);

CREATE TABLE IF NOT EXISTS sqlator_orders (
  order_id int PRIMARY KEY,
  order_number int NOT NULL,
  person_id int,
  total_amount decimal(10, 2),
  FOREIGN KEY (person_id) REFERENCES sqlator_persons (person_id)
);

CREATE TABLE IF NOT EXISTS sqlator_paged_items (
  id int PRIMARY KEY,
  n int NOT NULL
);

INSERT IGNORE INTO sqlator_persons (person_id, last_name, first_name)
VALUES (1, 'Smith', 'John'), (2, 'Garcia', 'Maria');

INSERT IGNORE INTO sqlator_orders (order_id, order_number, person_id, total_amount)
VALUES (101, 55001, 1, 150.75);

INSERT IGNORE INTO sqlator_paged_items (id, n)
SELECT n, 1251 - n FROM (
  SELECT ones.i + tens.i * 10 + hundreds.i * 100 + thousands.i * 1000 AS n
  FROM (SELECT 0 AS i UNION SELECT 1 UNION SELECT 2 UNION SELECT 3 UNION SELECT 4
        UNION SELECT 5 UNION SELECT 6 UNION SELECT 7 UNION SELECT 8 UNION SELECT 9) ones
  CROSS JOIN (SELECT 0 AS i UNION SELECT 1 UNION SELECT 2 UNION SELECT 3 UNION SELECT 4
        UNION SELECT 5 UNION SELECT 6 UNION SELECT 7 UNION SELECT 8 UNION SELECT 9) tens
  CROSS JOIN (SELECT 0 AS i UNION SELECT 1 UNION SELECT 2 UNION SELECT 3 UNION SELECT 4
        UNION SELECT 5 UNION SELECT 6 UNION SELECT 7 UNION SELECT 8 UNION SELECT 9) hundreds
  CROSS JOIN (SELECT 0 AS i UNION SELECT 1) thousands
) gen
WHERE n BETWEEN 1 AND 1250;
