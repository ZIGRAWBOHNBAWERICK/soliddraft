/** Copy and page data. Keep facts real; see design-brief.md. Change words here, not in the route. */
import type { ChapterId } from "./night/types";

export const themeColor = "#07080a";

/** Site-relative asset path with the deploy base in front (`/soliddraft/` on GitHub Pages). */
export const asset = (path: string) => `${import.meta.env.BASE_URL}${path.replace(/^\//, "")}`;

/** The released version comes from the Pages workflow (VITE_SOLIDDRAFT_VERSION, from the tag) or the workspace Cargo.toml. */
const releasedVersion = (import.meta.env.VITE_SOLIDDRAFT_VERSION as string | undefined) || "0.1.0";
const prerelease = (import.meta.env.VITE_SOLIDDRAFT_PRERELEASE as string | undefined) !== "false";

/** The public address of this site: the GitHub Pages address of the repository, with the browser app under `app/`. */
const publicUrl = "https://zigrawbohnbawerick.github.io/soliddraft/";

export const site = {
  name: "SolidDraft",
  shortName: "SolidDraft",
  version: `${releasedVersion}${prerelease ? " alpha" : ""}`,
  title: "SolidDraft: free, open-source parametric CAD on an exact kernel",
  description:
    "SolidDraft is a free, open-source parametric CAD app for mechanical parts. It runs on the desktop and in the browser, on Catallax, an exact geometry kernel written in Rust. MIT licence.",
  publicUrl,
  links: {
    /** The browser app: the released WebAssembly build, served beside this page. One setting. */
    app: `${publicUrl}app/`,
    releases: "https://github.com/ZIGRAWBOHNBAWERICK/soliddraft/releases",
    repo: "https://github.com/ZIGRAWBOHNBAWERICK/soliddraft",
    catallax: "https://github.com/BohnBawerick/catallax",
    kernelSite: "https://kernel.zigrawbohnbawerick.org/",
    sponsors: "https://github.com/sponsors/BohnBawerick",
    kofi: "https://ko-fi.com/pedropaiva56920",
    company: "https://zigrawbohnbawerick.org/",
  },
  company: "Zigrawbohnbawerick Pty Ltd",
  credit: "Made by Zigrawbohnbawerick Pty Ltd. MIT licence.",
};

export const accents = {
  oxide: "#ff5b1f",
  emerald: "#35c27c",
  ice: "#5bc8f5",
  violet: "#7660d6",
  amber: "#f2b134",
  bone: "#f2f0ea",
  ink: "#0b0c0d",
};

export const hero = {
  kicker: `Free, open-source CAD · ${site.version}`,
  title: "SolidDraft",
  line: "Parametric CAD for mechanical parts. It runs on your desktop and in your browser, on Catallax, an exact geometry kernel. Free to use, change and share, for ever.",
  actions: [
    { label: "Open in the browser", href: site.links.app, primary: true },
    { label: "Download", href: site.links.releases },
    { label: "Source", href: site.links.repo },
  ],
};

/** One page section. `span` is its scroll height in viewport heights; `flow` sections size to their content. */
export interface Section {
  id: string;
  nav: string;
  chapter?: ChapterId;
  span?: number;
  flow?: boolean;
  theme?: "light";
  veil?: boolean;
}

export const sections: Section[] = [
  { id: "intro", nav: "Intro", chapter: "hero", span: 170 },
  { id: "kernel", nav: "Kernel", chapter: "kernel", span: 300 },
  { id: "model", nav: "Model", chapter: "cad", span: 300 },
  { id: "drawings", nav: "Drawings", chapter: "draw", span: 280 },
  { id: "anywhere", nav: "Desktop and browser", chapter: "screen", span: 300 },
  { id: "statement", nav: "Desktop and browser", span: 110, veil: true },
  { id: "features", nav: "What works", chapter: "light", span: 260, theme: "light" },
  { id: "get", nav: "Get it", chapter: "smoke", flow: true },
  { id: "about", nav: "About", chapter: "smoke", flow: true },
];

export const rail = sections
  .filter((s, i, all) => all.findIndex((x) => x.nav === s.nav) === i)
  .map((s) => ({ id: s.id, label: s.nav }));

/** A caption shows while its chapter is active and progress sits inside [from, to]. */
export interface Caption {
  chapter: ChapterId;
  from: number;
  to: number;
  title: string;
  line: string;
  items?: string[];
}

export const captions: Caption[] = [
  {
    chapter: "kernel",
    from: 0,
    to: 0.44,
    title: "Every part is proven, not estimated.",
    line: "SolidDraft runs on Catallax, a geometry kernel in safe Rust. A STEP export states how far the file can lie from the exact part, and that number is proven. No closed kernel is rented.",
  },
  {
    chapter: "kernel",
    from: 0.56,
    to: 1,
    title: "Lattices and gyroids on the same part.",
    line: "A solid can end in a cubic lattice or a network gyroid, graded along an axis, and export a certified mesh for printing.",
  },
  {
    chapter: "cad",
    from: 0,
    to: 0.5,
    title: "Sketch, then extrude.",
    line: "Draw on a plane or a planar face: lines, rectangles, circles, arcs and slots, with relations and dimensions. Extrude or cut, blind, through all or up to a face.",
  },
  {
    chapter: "cad",
    from: 0.62,
    to: 1,
    title: "Holes, fillets, chamfers, patterns.",
    line: "Drill holes with counterbores and countersinks. Round and bevel edges, draft faces, shell a body, pattern and mirror it. Edit a feature later and the part rebuilds from the first change.",
  },
  {
    chapter: "draw",
    from: 0,
    to: 0.5,
    title: "A drawing from the same part.",
    line: "ISO and ANSI sheets with front, top, side and isometric views, hidden lines removed, and the model's dimensions brought in.",
  },
  {
    chapter: "draw",
    from: 0.6,
    to: 1,
    title: "SVG, PDF and DXF out.",
    line: "Sections and details fold off the base view. Every exported curve says whether it is the model's exact line or circle, or a mesh polyline.",
  },
  {
    chapter: "screen",
    from: 0,
    to: 0.5,
    title: "On your desktop.",
    line: "A feature tree, a ribbon, a GPU viewport and a command line that opens an assistant session. Alpha builds for Linux, Windows and macOS are on the releases page.",
    items: ["Linux AppImage and .deb", "Windows .msi", "macOS .dmg"],
  },
  {
    chapter: "screen",
    from: 0.62,
    to: 1,
    title: "And in your browser.",
    line: "The same app and the same kernel compile to one WebAssembly module. Open the address, draw a part, download the file. Nothing to install.",
  },
];

export const statement = {
  kicker: `SolidDraft · ${site.version}`,
  title: ["Free parametric CAD", "on an exact kernel."],
  cta: { label: "Open SolidDraft", href: site.links.app },
};

export interface FeatureItem {
  name: string;
  kind: string;
  line: string;
  href: string;
  /** Not built yet. Shown with a label, never as a working feature. */
  inDesign?: boolean;
}

export const features = {
  title: "What works today",
  line: "Everything below runs in the current build. The last row is in design and does not work yet.",
  items: [
    {
      name: "Sketches and features",
      kind: "Modelling",
      line: "Sketches with relations and dimensions. Extrude, cut, revolve and boolean, with persistent references, undo and redo.",
      href: `${site.links.repo}#what-works-today`,
    },
    {
      name: "Holes, patterns and mirrors",
      kind: "Modelling",
      line: "Blind or through holes with drill points, counterbores and countersinks at standard angles. Linear and circular patterns.",
      href: `${site.links.repo}#what-works-today`,
    },
    {
      name: "Fillets, chamfers, drafts and shells",
      kind: "Finish",
      line: "Fillets and chamfers between planes and cylinders, drafts of planar faces, planar shells with open faces and a set wall.",
      href: `${site.links.repo}/blob/main/docs/needs-catallax.md`,
    },
    {
      name: "Drawings",
      kind: "Output",
      line: "ISO and ANSI sheets, aligned views, sections, details, dimensions and annotations. SVG, PDF and DXF.",
      href: `${site.links.repo}/blob/main/docs/exchange.md`,
    },
    {
      name: "STEP, STL, 3MF and OBJ",
      kind: "Output",
      line: "STEP AP242 with a proven bound on its distance from the exact part. Meshes with a proven chord bound.",
      href: `${site.links.repo}/blob/main/docs/exchange.md`,
    },
    {
      name: "Assistant and MCP",
      kind: "Agents",
      line: "A Claude session in the command line proposes changes as cards you preview, apply and undo. Outside agents get the same commands over MCP.",
      href: `${site.links.repo}/blob/main/docs/assistant.md`,
    },
    {
      name: "Assemblies",
      kind: "Documents",
      line: "Insert parts from files, place them by position and rotation, save and reopen the assembly beside the part.",
      href: `${site.links.repo}/blob/main/docs/assemblies.md`,
    },
    {
      name: "General fillets, sweeps, lofts, STEP import, mates",
      kind: "In design",
      line: "Not built yet. Each waits on a named stage of the kernel's roadmap, and the ribbon shows it disabled with that stage.",
      href: `${site.links.repo}/blob/main/docs/roadmap.md`,
      inDesign: true,
    },
  ] satisfies FeatureItem[],
};

export interface GetItem {
  id: string;
  title: string;
  tags: string[];
  href: string;
  poster: string;
  /** A real screenshot of the app, or a render of a real part. */
  alt: string;
  video?: string;
  blurb: string;
  status?: string;
}

export const get: GetItem[] = [
  {
    id: "browser",
    title: "Open in the browser",
    tags: ["WebAssembly", "Nothing to install"],
    href: site.links.app,
    poster: asset("assets/app/browser-bracket.webp"),
    alt: "SolidDraft running in a web browser with the bracket open",
    blurb: "The kernel runs inside your tab. Files open by upload and save by download; the server keeps nothing.",
    status: site.version,
  },
  {
    id: "desktop",
    title: "Download for the desktop",
    tags: ["Linux", "Windows", "macOS"],
    href: site.links.releases,
    poster: asset("assets/app/desktop-bracket-dark.webp"),
    alt: "The SolidDraft desktop app with a bracket open and its feature tree",
    blurb: "AppImage, Debian package, Windows installer and macOS disk image on the GitHub releases page, unsigned for now, with checksums and build attestations.",
    status: site.version,
  },
  {
    id: "source",
    title: "Read the source",
    tags: ["Rust", "MIT"],
    href: site.links.repo,
    poster: asset("assets/work/bracket.jpg"),
    alt: "A rendered bracket exported from SolidDraft",
    video: asset("assets/work/bracket.mp4"),
    blurb: "Pull requests are welcome. Agent-written code is fine and expected; say so in the pull request and stay accountable for what it does.",
  },
  {
    id: "kernel",
    title: "Catallax, the kernel",
    tags: ["Kernel", "Rust", "B-rep", "Fields"],
    href: site.links.catallax,
    poster: asset("assets/work/lattice.jpg"),
    alt: "A rendered lattice block exported from SolidDraft",
    video: asset("assets/work/lattice.mp4"),
    blurb: "Exact B-rep and signed distance fields on one recipe. SolidDraft and Catallax are one project in two repositories.",
  },
];

export const about = {
  title: "Free to use, change and share.",
  line: "SolidDraft is MIT licensed. The licence keeps one thing: the credit line. Alpha software: keep backups of your documents. Donations fund the work; both buttons support the same project.",
  /** The donation buttons. Both addresses stay as they are. */
  support: [
    { label: "Support on Ko-fi", href: site.links.kofi, primary: true },
    { label: "GitHub Sponsors", href: site.links.sponsors },
  ],
  links: [
    { label: "Open the app", href: site.links.app },
    { label: "Releases", href: site.links.releases },
    { label: "Repository", href: site.links.repo },
    { label: "Catallax", href: site.links.catallax },
    { label: "GitHub Sponsors", href: site.links.sponsors },
    { label: "Ko-fi", href: site.links.kofi },
  ],
  foot: {
    credit: site.credit,
    company: `Free tools and paid apps from ${site.company}.`,
    other: { label: "Other products", href: site.links.company },
  },
};
