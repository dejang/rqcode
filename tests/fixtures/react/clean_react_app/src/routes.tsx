import React from "react";

export const SettingsPage = React.lazy(() => import("./pages/Settings"));

export const routes = [
  {
    path: "/billing",
    element: React.lazy(() => import("./pages/Billing")),
  },
];
