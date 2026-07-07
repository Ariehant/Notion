/**
 * Single source of truth for the site's brand + links.
 *
 * NOTE: the product is currently named "Notion", which collides with Notion
 * Labs' trademark. To rebrand the public site, change `productName` (and
 * optionally `tagline`) here — nothing else references the name directly.
 */
export const site = {
  productName: "Notion",
  // Disambiguates from the commercial Notion.
  tagline: "The offline-first, encrypted notes app.",
  githubUrl: "https://github.com/Ariehant/Notion",
  releasesUrl: "https://github.com/Ariehant/Notion/releases",
  docsUrl: "https://github.com/Ariehant/Notion/blob/main/docs/OPEN_NOTEBOOK.md",
  version: "0.1.0",
} as const;
