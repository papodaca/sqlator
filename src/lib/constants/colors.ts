import type { ConnectionColorId } from "$lib/types";

export const CONNECTION_COLORS: readonly {
  id: ConnectionColorId;
  hex: string;
  label: string;
}[] = [
  { id: "red", hex: "#ef4444", label: "Red" },
  { id: "orange", hex: "#f97316", label: "Orange" },
  { id: "yellow", hex: "#eab308", label: "Yellow" },
  { id: "green", hex: "#22c55e", label: "Green" },
  { id: "teal", hex: "#14b8a6", label: "Teal" },
  { id: "blue", hex: "#3b82f6", label: "Blue" },
  { id: "violet", hex: "#8b5cf6", label: "Violet" },
  { id: "pink", hex: "#ec4899", label: "Pink" },
  { id: "slate", hex: "#64748b", label: "Slate" },
  { id: "white", hex: "#f8fafc", label: "White" },
] as const;

export function getColorHex(colorId: string): string {
  return (
    CONNECTION_COLORS.find((c) => c.id === colorId)?.hex ?? "#64748b"
  );
}
