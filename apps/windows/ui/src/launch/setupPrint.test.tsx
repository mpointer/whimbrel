import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

// The print page is drawn through a portal, which a static render cannot show: record what the screen hands it.
type PrintProps = { words: readonly string[]; copy?: { notice: string; footer: string } };
const seen = vi.hoisted(() => ({ props: null as unknown as PrintProps | null }));
vi.mock("./RecoveryWords", async (actual) => ({
  ...(await actual<typeof import("./RecoveryWords")>()),
  PrintPortal: (props: { words: readonly string[]; copy?: { notice: string; footer: string } }) => {
    seen.props = props;
    return null;
  },
}));

const { SetupModel } = await import("./setupModel");
const { SetupView } = await import("./SetupView");

const WORDS = Array.from({ length: 24 }, (_, i) => `word${i + 1}`);
const last = () => seen.props as PrintProps | null;
async function made() {
  const model = new SetupModel({
    begin: async () => ({ kind: "made", words: WORDS }),
    saveFile: vi.fn(),
    verify: vi.fn(),
    startOver: vi.fn(),
    print: vi.fn(),
    reusesKey: false,
  } as never);
  model.setPassphrase("a long enough passphrase");
  model.setConfirmation("a long enough passphrase");
  await model.create();
  return model;
}

describe("the setup screen hands its print words to the print page", () => {
  it("the redesigned card's own words reach the printed page", async () => {
    seen.props = null;
    const words = { intro: "I", sameDisk: "S", wordsWarning: "W", footer: "F", printNotice: "PRINT NOTICE", printFooter: "PRINT FOOTER" };
    renderToStaticMarkup(<SetupView model={await made()} continueTitle="Encrypt" onContinue={() => {}} embedded words={words} />);
    expect(last()?.copy).toEqual({ notice: "PRINT NOTICE", footer: "PRINT FOOTER" });
  });

  it("the launch screen and the old tab pass none, so their page is unchanged", async () => {
    seen.props = null;
    renderToStaticMarkup(<SetupView model={await made()} continueTitle="Continue" onContinue={() => {}} />);
    expect(last()).not.toBeNull();
    expect(last()?.copy).toBeUndefined();
  });
});
