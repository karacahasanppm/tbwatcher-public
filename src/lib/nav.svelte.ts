// App-wide tab state, so one tab can send the user to another — e.g. the stash opening an item in Movers.
export type Tab = "market" | "movers" | "portfolio" | "alerts" | "stash" | "gear";

export const nav = $state<{ tab: Tab; moversQuery: string }>({ tab: "market", moversQuery: "" });

/** Switch to Movers searching for `name`; Movers consumes the query when it mounts. */
export function openInMovers(name: string) {
  nav.moversQuery = name;
  nav.tab = "movers";
}
