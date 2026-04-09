import { api } from "$lib/api";
import type { TableMeta } from "$lib/types";

export async function fetchSchemaMetadata(
  connectionId: string,
  sql: string,
): Promise<TableMeta | null> {
  try {
    const result = await api.invoke<TableMeta | null>("fetch_schema_metadata", {
      connectionId,
      sql,
    });
    return result;
  } catch {
    // Schema fetch failure is non-fatal — grid stays read-only
    return null;
  }
}
