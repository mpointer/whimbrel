// No-store mode (Mike's decision 1, brief 2.12, 3.3.8): after a failed plain open the user may go on
// without the memory database. Recording, capture and store-dependent sync stay off; the shell
// refuses them, and the UI says so and disables the start controls.

import { createContext, useContext } from "react";

/** True while Whimbrel runs without the memory database. Set by the LaunchGate. */
export const NoStoreContext = createContext(false);

export function useNoStore(): boolean {
  return useContext(NoStoreContext);
}
