// learn.xabcnews.com: copy buttons on code, the section being read marked on the right, the
// current lesson brought into view on the left, and the way back to the top.
(() => {
  "use strict";

  for (const pre of document.querySelectorAll(".prose pre")) {
    const code = pre.querySelector("code");
    if (!code) continue;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "copy";
    button.textContent = "复制";
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(code.textContent);
        button.textContent = "已复制";
        button.dataset.done = "";
      } catch {
        button.textContent = "复制失败";
      }
      setTimeout(() => {
        button.textContent = "复制";
        delete button.dataset.done;
      }, 1600);
    });
    pre.append(button);
  }

  // The left column scrolls by itself, not the page.
  const side = document.querySelector(".side");
  const current = side?.querySelector("[aria-current]");
  if (side && current) {
    side.scrollTop = current.offsetTop - side.clientHeight / 3;
  }

  const links = new Map(
    [...document.querySelectorAll(".aside a[href^='#']")].map((link) => [link.hash.slice(1), link]),
  );
  if (links.size) {
    const headings = [...links.keys()].map((id) => document.getElementById(id)).filter(Boolean);
    const mark = () => {
      let active = headings[0];
      for (const heading of headings) {
        if (heading.getBoundingClientRect().top < 120) active = heading;
      }
      for (const [id, link] of links) {
        if (id === active?.id) link.setAttribute("aria-current", "true");
        else link.removeAttribute("aria-current");
      }
    };
    addEventListener("scroll", mark, { passive: true });
    mark();
  }

  const top = document.querySelector(".to-top");
  if (top) {
    const toggle = () => { top.hidden = scrollY < innerHeight; };
    addEventListener("scroll", toggle, { passive: true });
    toggle();
    top.addEventListener("click", () => {
      scrollTo({ top: 0 });
      document.querySelector(".brand")?.focus({ preventScroll: true });
    });
  }
})();
