import { invoke } from "@tauri-apps/api/core";
import type { ConnectionGroup, ConnectionInfo } from "$lib/types";

let groupList = $state<ConnectionGroup[]>([]);

export const groups = {
  get list() {
    return groupList;
  },

  byId(id: string): ConnectionGroup | undefined {
    return groupList.find((g) => g.id === id);
  },

  async load() {
    try {
      groupList = await invoke<ConnectionGroup[]>("get_groups");
    } catch (e) {
      console.error("Failed to load groups:", e);
    }
  },

  async create(
    name: string,
    color: string | null = null,
    parentGroupId: string | null = null,
  ): Promise<ConnectionGroup> {
    const group = await invoke<ConnectionGroup>("save_group", {
      payload: { name, color, parent_group_id: parentGroupId },
    });
    groupList = [...groupList, group];
    return group;
  },

  async update(group: ConnectionGroup): Promise<ConnectionGroup> {
    const updated = await invoke<ConnectionGroup>("update_group", { group });
    groupList = groupList.map((g) => (g.id === group.id ? updated : g));
    return updated;
  },

  async remove(id: string): Promise<void> {
    await invoke("delete_group", { id });
    groupList = groupList.filter((g) => g.id !== id);
  },

  async toggleCollapsed(id: string): Promise<void> {
    const group = groupList.find((g) => g.id === id);
    if (!group) return;
    const updated = { ...group, collapsed: !group.collapsed };
    await invoke<ConnectionGroup>("update_group", { group: updated });
    groupList = groupList.map((g) => (g.id === id ? updated : g));
  },

  async moveConnection(
    connectionId: string,
    groupId: string | null,
  ): Promise<ConnectionInfo> {
    return await invoke<ConnectionInfo>("move_connection_to_group", {
      connectionId,
      groupId,
    });
  },

  /** Returns root-level groups (no parent), sorted by order. */
  get roots(): ConnectionGroup[] {
    return groupList
      .filter((g) => !g.parent_group_id)
      .sort((a, b) => a.order - b.order || a.name.localeCompare(b.name));
  },

  /** Returns children of a given group id, sorted by order. */
  childrenOf(parentId: string): ConnectionGroup[] {
    return groupList
      .filter((g) => g.parent_group_id === parentId)
      .sort((a, b) => a.order - b.order || a.name.localeCompare(b.name));
  },

  /** Compute nesting depth of a group (root = 0). */
  depthOf(groupId: string): number {
    let depth = 0;
    let current: ConnectionGroup | undefined = groupList.find(
      (g) => g.id === groupId,
    );
    while (current?.parent_group_id) {
      depth++;
      current = groupList.find((g) => g.id === current!.parent_group_id);
    }
    return depth;
  },
};
