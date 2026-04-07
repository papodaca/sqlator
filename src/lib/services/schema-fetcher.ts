import { invoke } from "@tauri-apps/api/core";
import type { TableMeta } from "$lib/types";

export async function fetchSchemaMetadata(
  connectionId: string,
  sql: string,
): Promise<TableMeta | null> {
  try {
    const result = await invoke<TableMeta | null>("fetch_schema_metadata", {
      connectionId,
      sql,
    });
    return result;
  } catch {
    // Schema fetch failure is non-fatal — grid stays read-only
    return null;
  }
}
