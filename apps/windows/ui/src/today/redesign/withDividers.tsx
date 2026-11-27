// The divider rule between Home's parts. Apart from HomeView so a skin card's picture of Home can use it without the live Home.

import { Fragment, type ReactNode } from "react";
import { WDivider } from "../../system";

/** The parts of Home in order, with the skin's divider above each one after the first. A part that draws nothing (null) takes no
 *  divider. With no divider style the parts come back as they were, so Thread's markup does not change. */
export function withDividers(parts: ReactNode[], on: boolean): ReactNode[] {
  return parts
    .filter((part) => part != null && part !== false)
    .map((part, i) => (
      <Fragment key={i}>
        {on && i > 0 ? <WDivider /> : null}
        {part}
      </Fragment>
    ));
}
