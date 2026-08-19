-- Dedicated sqlator_* tables. Guard inserts with count() (MergeTree is not unique).

CREATE TABLE IF NOT EXISTS sqlator_persons (
  person_id Int32,
  last_name String,
  first_name String
) ENGINE = MergeTree
ORDER BY person_id;

CREATE TABLE IF NOT EXISTS sqlator_orders (
  order_id Int32,
  order_number Int32,
  person_id Int32,
  total_amount Decimal(10, 2)
) ENGINE = MergeTree
ORDER BY order_id;

CREATE TABLE IF NOT EXISTS sqlator_paged_items (
  id Int32,
  n Int32
) ENGINE = MergeTree
ORDER BY id;

INSERT INTO sqlator_persons
SELECT 1, 'Smith', 'John'
WHERE (SELECT count() FROM sqlator_persons) = 0;

INSERT INTO sqlator_persons
SELECT 2, 'Garcia', 'Maria'
WHERE (SELECT count() FROM sqlator_persons) = 1;

INSERT INTO sqlator_orders
SELECT 101, 55001, 1, 150.75
WHERE (SELECT count() FROM sqlator_orders) = 0;

INSERT INTO sqlator_paged_items
SELECT number + 1, 1250 - number
FROM numbers(1250)
WHERE (SELECT count() FROM sqlator_paged_items) = 0;
