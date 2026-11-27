import React from "react";

export function PrimaryMenu({ activePath }: { activePath: string }) {
  const navClass = activePath === "/home" ? "nav-active" : "nav-default";
  return (
    <nav aria-label="Primary" className={navClass}>
      <ul>
        <li>Home</li>
        <li>Orders</li>
        <li>Invoices</li>
        <li>Customers</li>
        <li>Reports</li>
        <li>Settings</li>
        <li>Billing</li>
        <li>Support</li>
      </ul>
    </nav>
  );
}

export function FooterLinks({ showStatus }: { showStatus: boolean }) {
  const year = new Date().getFullYear();
  return (
    <footer aria-label="Legal">
      <small>{year}</small>
      {showStatus && <span>All systems operational</span>}
      <ul>
        <li>Privacy</li>
        <li>Terms</li>
        <li>Security</li>
        <li>Status</li>
        <li>Docs</li>
        <li>Contact</li>
        <li>Changelog</li>
        <li>Careers</li>
      </ul>
    </footer>
  );
}
