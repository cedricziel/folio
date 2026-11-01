import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { authScreens, config, tour, TourInstance, viewportsFor } from "./tour";

// Serious and critical axe violations fail the build. Known ones are listed
// here with their issue: matching nodes are ignored in the per-screen tests
// and each entry shows up as a fixme test, so they stay visible in reports.
// Remove an entry together with the fix; anything new still fails.
const known: { rule: string; target: string; issue: string }[] = [
  {
    rule: "color-contrast",
    target: ".search-label",
    issue: "https://github.com/cedricziel/folio/issues/64",
  },
  {
    rule: "target-size",
    target: ".f-tag",
    issue: "https://github.com/cedricziel/folio/issues/64",
  },
  {
    rule: "aria-required-children",
    target: ".palette-results",
    issue: "https://github.com/cedricziel/folio/issues/62",
  },
  {
    rule: "aria-required-parent",
    target: "data-index",
    issue: "https://github.com/cedricziel/folio/issues/62",
  },
  {
    rule: "aria-input-field-name",
    target: ".palette-results",
    issue: "https://github.com/cedricziel/folio/issues/62",
  },
  {
    rule: "listitem",
    target: ".palette-results",
    issue: "https://github.com/cedricziel/folio/issues/62",
  },
  {
    rule: "label",
    target: "email_to",
    issue: "https://github.com/cedricziel/folio/issues/63",
  },
  {
    rule: "select-name",
    target: "_mode",
    issue: "https://github.com/cedricziel/folio/issues/63",
  },
  {
    rule: "document-title",
    target: "html",
    issue: "https://github.com/cedricziel/folio/issues/60",
  },
  {
    rule: "html-has-lang",
    target: "html",
    issue: "https://github.com/cedricziel/folio/issues/60",
  },
];

const isKnown = (rule: string, target: string) =>
  known.some((k) => k.rule === rule && target.includes(k.target));

// The first test creates the owner and seeds seven documents.
test.describe.configure({ timeout: 90_000 });

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

for (const k of known) {
  test(`known axe violation: ${k.rule} on ${k.target}`, () => {
    test.fixme(true, k.issue);
  });
}

for (const screen of authScreens) {
  for (const viewport of viewportsFor(screen, ["desktop", "mobile"])) {
    for (const theme of config.themes) {
      test(`axe: ${screen.name} ${viewport.name} ${theme}`, async ({
        browser,
      }) => {
        if (screen.phase === "empty" && screen.auth === "on") {
          // /setup only renders before the owner exists.
          test.skip(instance.ownerReady(), "owner already created");
        } else {
          await instance.ready(browser, { seeded: screen.phase === "seeded" });
        }
        if (screen.name === "inbox-empty")
          test.skip(
            await instance.hasDocuments(browser),
            "instance already seeded",
          );

        const context = await instance.context(
          browser,
          screen,
          viewport,
          theme,
        );
        const page = await context.newPage();
        await tour.openScreen(page, screen);
        const results = await new AxeBuilder({ page })
          .withTags(tour.axeTags)
          .analyze();
        const blocking = results.violations
          .filter((v) => v.impact === "serious" || v.impact === "critical")
          .flatMap((v) =>
            v.nodes
              .map((n) => n.target.join(" "))
              .filter((target) => !isKnown(v.id, target))
              .map((target) => `${v.id} (${v.impact}): ${target} - ${v.help}`),
          );
        expect(
          blocking,
          `${screen.name}: serious/critical axe violations`,
        ).toEqual([]);
        expect(
          await tour.hasHorizontalOverflow(page),
          "page scrolls horizontally",
        ).toBe(false);
        await context.close();
      });
    }
  }
}
