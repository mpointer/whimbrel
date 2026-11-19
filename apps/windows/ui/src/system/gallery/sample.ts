// Fictional sample data for the gallery. Alex Rivera is the user. Nobody here is real.
import type { WMemoryItem } from "../layout";

export const PEOPLE = { alex: "Alex Rivera", sam: "Sam Okafor", priya: "Priya Shah", dana: "Dana Whitfield", jo: "Jo at Northwind" };

export const MEMORY_ITEMS: WMemoryItem[] = [
  { app: "green", icon: "table", name: "Pricing v3.xlsx in Excel" },
  { app: "purple", icon: "chat", name: "#pricing in Slack" },
  { app: "blue", icon: "mail", name: "2 emails with Northwind" },
];

export const SOURCES = [
  { number: 1, who: "Jo, Northwind", time: "06:12", quote: "If the price holds, we can do two years." },
  { number: 2, who: "Dana Whitfield", time: "09:40", quote: "We expect fifteen more seats by Q2. I will confirm the date." },
  { number: 3, who: "Jo, Northwind", time: "18:25", quote: "There are still a few open items on the questionnaire." },
  { number: 4, who: "Sam Okafor", time: "28:50", quote: "I will get the quote to you by Thursday." },
  { number: 5, who: "Dana Whitfield", time: "29:31", quote: "I will answer the open items by Friday." },
  { number: 6, who: "Alex Rivera", time: "31:02", quote: "I will check the seat price before we send it." },
];

export const NOTE = {
  title: "Northwind renewal call",
  meta: "Monday, November 16 · 32 min",
  people: "Sam Okafor, Dana Whitfield, Jo (Northwind), you",
  yours: ["jo ok with 2 yr term if price holds", "security questionnaire?? who owns it", "seats 40 to 55 in Q2"],
  renewal: [
    { text: "Northwind will renew for two years if the price stays the same.", sources: [1] },
    { text: "Seats grow from 40 to 55 in Q2. Dana will confirm the start date.", sources: [2] },
  ],
  risk: { text: "Northwind’s security questionnaire still has open items, and the renewal date is close.", sources: [3] },
  steps: [
    { text: "Send the two-year renewal quote to Jo.", owner: "Sam Okafor", due: "Due Thu, Nov 19", sources: [4], at: "28:50" },
    { text: "Answer the open items on the questionnaire.", owner: "Dana Whitfield", due: "Due Fri, Nov 20", sources: [5], at: "29:31" },
    { text: "Check that 55 seats still fits the current price.", owner: "Alex Rivera", due: "Due Tue, Nov 24", sources: [6], at: "31:02" },
  ],
};

export const TRANSCRIPT = [
  { time: "10:12", speaker: "Priya Shah", text: "So a usage-based tier, on top of seats?" },
  { time: "10:12", speaker: "Sam Okafor", text: "Yes. Small base, then pay for what you use. Starter would like it most." },
  { time: "10:13", speaker: "Alex Rivera", text: "Do we know what that does to churn on Starter?" },
  { time: "10:14", speaker: "Sam Okafor", text: "On the annual discount, I would start at 15 and see" },
];

export const PAUSE_CHOICES = [
  { id: "15", label: "Pause for 15 minutes", detail: "Back on at 10:07 AM" },
  { id: "60", label: "Pause for 1 hour", detail: "Back on at 10:52 AM" },
  { id: "tomorrow", label: "Pause until tomorrow", detail: "Back on Wed" },
];
