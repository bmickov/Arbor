// Links to where a repository is hosted: commits, branches and files on the
// web, and the page that starts a pull request (merge request on GitLab).
// Pure functions of a remote's URL, so they can be tested on their own.

export type HostKind = "github" | "gitlab" | "bitbucket" | "gitea" | "azure";

export interface Host {
  kind: HostKind;
  /** The repository's web address, e.g. https://github.com/owner/repo */
  web: string;
  /** For menus: "GitHub", "GitLab", "Bitbucket", "Codeberg", "Gitea", "Azure DevOps". */
  name: string;
  /** "pull request", or "merge request" on GitLab. */
  pr: string;
}

/** The web address of a hosted remote: git@github.com:o/r.git → https://github.com/o/r */
export function webUrl(remote: string): string | null {
  const m =
    remote.match(/^git@([^:]+):(.+?)(?:\.git)?\/?$/) ??
    remote.match(/^ssh:\/\/(?:[^@]+@)?([^/:]+)(?::\d+)?\/(.+?)(?:\.git)?\/?$/) ??
    remote.match(/^https?:\/\/(?:[^@/]+@)?([^/]+)\/(.+?)(?:\.git)?\/?$/);
  return m ? `https://${m[1]}/${m[2]}` : null;
}

/** Recognise the hosting service from a remote URL; null for anything else. */
export function hostOf(remote: string): Host | null {
  // Azure DevOps has its own URL shapes.
  const az =
    remote.match(/^(?:https?:\/\/)?(?:[^@/]+@)?dev\.azure\.com\/([^/]+)\/([^/]+)\/_git\/([^/?#]+?)\/?$/) ??
    remote.match(/^(?:ssh:\/\/)?git@ssh\.dev\.azure\.com:(?:22\/)?v3\/([^/]+)\/([^/]+)\/([^/]+?)\/?$/);
  if (az) {
    return { kind: "azure", web: `https://dev.azure.com/${az[1]}/${az[2]}/_git/${az[3]}`, name: "Azure DevOps", pr: "pull request" };
  }
  const web = webUrl(remote);
  if (!web) return null;
  const host = new URL(web).hostname.toLowerCase();
  if (host === "github.com" || host.endsWith(".github.com") || /(^|\.)github\./.test(host)) {
    return { kind: "github", web, name: "GitHub", pr: "pull request" };
  }
  if (host === "gitlab.com" || /(^|\.)gitlab\./.test(host)) {
    return { kind: "gitlab", web, name: "GitLab", pr: "merge request" };
  }
  if (host === "bitbucket.org") return { kind: "bitbucket", web, name: "Bitbucket", pr: "pull request" };
  if (host === "codeberg.org") return { kind: "gitea", web, name: "Codeberg", pr: "pull request" };
  if (/(^|\.)(gitea|forgejo)\./.test(host)) return { kind: "gitea", web, name: host.includes("forgejo") ? "Forgejo" : "Gitea", pr: "pull request" };
  return null;
}

/** Encode a branch or path for a URL, keeping its slashes. */
const seg = (s: string) => s.split("/").map(encodeURIComponent).join("/");
const q = encodeURIComponent;

export function commitUrl(h: Host, oid: string): string {
  switch (h.kind) {
    case "gitlab": return `${h.web}/-/commit/${oid}`;
    case "bitbucket": return `${h.web}/commits/${oid}`;
    default: return `${h.web}/commit/${oid}`;
  }
}

export function branchUrl(h: Host, branch: string): string {
  switch (h.kind) {
    case "gitlab": return `${h.web}/-/tree/${seg(branch)}`;
    case "bitbucket": return `${h.web}/src/${seg(branch)}`;
    case "gitea": return `${h.web}/src/branch/${seg(branch)}`;
    case "azure": return `${h.web}?version=GB${q(branch)}`;
    default: return `${h.web}/tree/${seg(branch)}`;
  }
}

/** A file as it is at a commit. */
export function fileUrl(h: Host, oid: string, path: string, line?: number): string {
  const l = line ? (h.kind === "bitbucket" ? `#lines-${line}` : h.kind === "azure" ? `&line=${line}` : `#L${line}`) : "";
  switch (h.kind) {
    case "gitlab": return `${h.web}/-/blob/${oid}/${seg(path)}${l}`;
    case "bitbucket": return `${h.web}/src/${oid}/${seg(path)}${l}`;
    case "gitea": return `${h.web}/src/commit/${oid}/${seg(path)}${l}`;
    case "azure": return `${h.web}?path=${q("/" + path)}&version=GC${oid}${l}`;
    default: return `${h.web}/blob/${oid}/${seg(path)}${l}`;
  }
}

/** The page that opens a new pull (merge) request from `branch` into `base`. */
export function newPullRequestUrl(h: Host, branch: string, base?: string | null): string {
  switch (h.kind) {
    case "gitlab":
      return `${h.web}/-/merge_requests/new?merge_request%5Bsource_branch%5D=${q(branch)}` + (base ? `&merge_request%5Btarget_branch%5D=${q(base)}` : "");
    case "bitbucket":
      return `${h.web}/pull-requests/new?source=${q(branch)}` + (base ? `&dest=${q(base)}` : "");
    case "azure":
      return `${h.web}/pullrequestcreate?sourceRef=${q(branch)}` + (base ? `&targetRef=${q(base)}` : "");
    case "gitea":
      return base ? `${h.web}/compare/${seg(base)}...${seg(branch)}` : `${h.web}/compare/${seg(branch)}`;
    default:
      return base ? `${h.web}/compare/${seg(base)}...${seg(branch)}?expand=1` : `${h.web}/pull/new/${seg(branch)}`;
  }
}

/** The repository's list of open pull (merge) requests. */
export function pullRequestsUrl(h: Host): string {
  switch (h.kind) {
    case "gitlab": return `${h.web}/-/merge_requests`;
    case "bitbucket": return `${h.web}/pull-requests`;
    case "azure": return `${h.web}/pullrequests`;
    default: return `${h.web}/pulls`;
  }
}
