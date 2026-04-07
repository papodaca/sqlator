import type { ChangeSet, TableMeta } from "$lib/types";

export interface ValidationError {
  rowId: string;
  column: string;
  message: string;
}

export function validateChangeSet(
  changeSet: ChangeSet,
  tableMeta: TableMeta,
): ValidationError[] {
  const errors: ValidationError[] = [];

  for (const [tempId, addedRow] of changeSet.added) {
    for (const col of tableMeta.columns) {
      if (col.isAutoIncrement || col.isGenerated) continue;
      const value = addedRow.data[col.name];
      const hasValue = col.name in addedRow.data && value !== undefined;

      if (!col.nullable && !col.isAutoIncrement && !col.isGenerated) {
        if (!hasValue || value === null) {
          errors.push({
            rowId: tempId,
            column: col.name,
            message: `Column "${col.name}" cannot be null`,
          });
        }
      }
    }
  }

  return errors;
}
