import type { AnchorHTMLAttributes } from "react";

export function navigate(path: string) {
  window.location.hash = path;
  window.scrollTo({ top: 0, behavior: "instant" });
}

const router = { push: navigate, replace: (path: string) => window.location.replace(`#${path}`) };
export const useRouter = () => router;

export default function Link({ href, onClick, ...props }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  return <a {...props} href={`#${href}`} onClick={(event) => {
    onClick?.(event);
    if (!event.defaultPrevented && !event.metaKey && !event.ctrlKey && event.button === 0) {
      event.preventDefault();
      navigate(href);
    }
  }} />;
}
