#!/usr/bin/env bash
# First-boot seed for the Microsoft SQL Server image (no docker-entrypoint-initdb.d).
set -euo pipefail
/opt/mssql/bin/sqlservr &
sqlpid=$!
sqlcmd=""
for candidate in /opt/mssql-tools18/bin/sqlcmd /opt/mssql-tools/bin/sqlcmd; do
  if [ -x "$candidate" ]; then
    sqlcmd="$candidate"
    break
  fi
done
if [ -z "$sqlcmd" ]; then
  echo "sqlcmd not found in image" >&2
  wait "$sqlpid"
  exit 1
fi
for _ in $(seq 1 60); do
  if "$sqlcmd" -C -S localhost -U sa -P "${MSSQL_SA_PASSWORD}" -Q "SELECT 1" >/dev/null 2>&1; then
    "$sqlcmd" -C -S localhost -U sa -P "${MSSQL_SA_PASSWORD}" -i /init/mssql.sql
    wait "$sqlpid"
    exit $?
  fi
  sleep 2
done
echo "MSSQL did not accept connections in time" >&2
wait "$sqlpid"
exit 1
