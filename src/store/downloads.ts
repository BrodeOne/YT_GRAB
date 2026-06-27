import { create } from "zustand";
import type { DownloadItem } from "../types";

interface DownloadStore {
  items: Record<string, DownloadItem>;
  addItem: (item: DownloadItem) => void;
  updateProgress: (
    id: string,
    progress: number,
    speed: string,
    eta: string,
    downloadedBytes: number,
    totalBytes: number | null,
  ) => void;
  markComplete: (id: string, filename?: string) => void;
  markError: (id: string, error: string) => void;
  markCancelled: (id: string) => void;
  removeItem: (id: string) => void;
  clearCompleted: () => void;
}

export const useDownloadStore = create<DownloadStore>((set) => ({
  items: {},

  addItem: (item) =>
    set((state) => ({
      items: { ...state.items, [item.id]: item },
    })),

  updateProgress: (id, progress, speed, eta, downloadedBytes, totalBytes) =>
    set((state) => {
      const item = state.items[id];
      if (!item) return state;
      return {
        items: {
          ...state.items,
          [id]: { ...item, progress, speed, eta, downloadedBytes, totalBytes },
        },
      };
    }),

  markComplete: (id: string, filename?: string) =>
    set((state) => {
      const item = state.items[id];
      if (!item) return state;
      return {
        items: { ...state.items, [id]: { ...item, status: "completed" as const, progress: 100, filename: filename ?? item.filename } },
      };
    }),

  markError: (id, error) =>
    set((state) => {
      const item = state.items[id];
      if (!item) return state;
      return {
        items: { ...state.items, [id]: { ...item, status: "error" as const, error } },
      };
    }),

  markCancelled: (id) =>
    set((state) => {
      const item = state.items[id];
      if (!item) return state;
      return {
        items: { ...state.items, [id]: { ...item, status: "cancelled" as const } },
      };
    }),

  removeItem: (id) =>
    set((state) => {
      const next = { ...state.items };
      delete next[id];
      return { items: next };
    }),

  clearCompleted: () =>
    set((state) => {
      const next: Record<string, DownloadItem> = {};
      for (const [id, item] of Object.entries(state.items)) {
        if (item.status === "downloading") {
          next[id] = item;
        }
      }
      return { items: next };
    }),
}));
