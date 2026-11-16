// The web build is only for quick previews: the browser's storage stands in.
export const getItem = async (k: string) => globalThis.localStorage?.getItem(k) ?? null;
export const setItem = async (k: string, v: string) => globalThis.localStorage?.setItem(k, v);
export const deleteItem = async (k: string) => globalThis.localStorage?.removeItem(k);
