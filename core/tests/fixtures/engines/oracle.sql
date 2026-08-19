-- Simple statements. oracle-rs rejects large PL/SQL blocks. First boot via sqlplus. Tests ignore ORA-00955.

CREATE TABLE sqlator_persons (
  person_id NUMBER PRIMARY KEY,
  last_name VARCHAR2(255) NOT NULL,
  first_name VARCHAR2(255)
);

CREATE TABLE sqlator_orders (
  order_id NUMBER PRIMARY KEY,
  order_number NUMBER NOT NULL,
  person_id NUMBER,
  total_amount NUMBER(10, 2),
  CONSTRAINT sqlator_orders_person_fk FOREIGN KEY (person_id) REFERENCES sqlator_persons (person_id)
);

CREATE TABLE sqlator_paged_items (
  id NUMBER PRIMARY KEY,
  n NUMBER NOT NULL
);

INSERT INTO sqlator_persons (person_id, last_name, first_name)
SELECT 1, 'Smith', 'John' FROM dual
WHERE NOT EXISTS (SELECT 1 FROM sqlator_persons WHERE person_id = 1);

INSERT INTO sqlator_persons (person_id, last_name, first_name)
SELECT 2, 'Garcia', 'Maria' FROM dual
WHERE NOT EXISTS (SELECT 1 FROM sqlator_persons WHERE person_id = 2);

INSERT INTO sqlator_orders (order_id, order_number, person_id, total_amount)
SELECT 101, 55001, 1, 150.75 FROM dual
WHERE NOT EXISTS (SELECT 1 FROM sqlator_orders WHERE order_id = 101);

INSERT INTO sqlator_paged_items (id, n)
SELECT lvl, 1251 - lvl FROM (
  SELECT LEVEL AS lvl FROM dual CONNECT BY LEVEL <= 1250
)
WHERE (SELECT COUNT(*) FROM sqlator_paged_items) = 0;