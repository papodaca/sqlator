-- Dedicated sqlator_* names. GO separates compile batches (CREATE then INSERT).

IF OBJECT_ID(N'dbo.sqlator_persons', N'U') IS NULL
CREATE TABLE dbo.sqlator_persons (
  person_id int PRIMARY KEY,
  last_name varchar(255) NOT NULL,
  first_name varchar(255)
);
GO

IF OBJECT_ID(N'dbo.sqlator_orders', N'U') IS NULL
CREATE TABLE dbo.sqlator_orders (
  order_id int PRIMARY KEY,
  order_number int NOT NULL,
  person_id int,
  total_amount decimal(10, 2),
  FOREIGN KEY (person_id) REFERENCES dbo.sqlator_persons (person_id)
);
GO

IF OBJECT_ID(N'dbo.sqlator_paged_items', N'U') IS NULL
CREATE TABLE dbo.sqlator_paged_items (
  id int PRIMARY KEY,
  n int NOT NULL
);
GO

IF NOT EXISTS (SELECT 1 FROM dbo.sqlator_persons WHERE person_id = 1)
INSERT INTO dbo.sqlator_persons (person_id, last_name, first_name) VALUES (1, 'Smith', 'John');

IF NOT EXISTS (SELECT 1 FROM dbo.sqlator_persons WHERE person_id = 2)
INSERT INTO dbo.sqlator_persons (person_id, last_name, first_name) VALUES (2, 'Garcia', 'Maria');

IF NOT EXISTS (SELECT 1 FROM dbo.sqlator_orders WHERE order_id = 101)
INSERT INTO dbo.sqlator_orders (order_id, order_number, person_id, total_amount) VALUES (101, 55001, 1, 150.75);

IF NOT EXISTS (SELECT 1 FROM dbo.sqlator_paged_items)
BEGIN
  WITH seq AS (
    SELECT 1 AS n
    UNION ALL
    SELECT n + 1 FROM seq WHERE n < 1250
  )
  INSERT INTO dbo.sqlator_paged_items (id, n)
  SELECT n, 1251 - n FROM seq
  OPTION (MAXRECURSION 1250);
END;
