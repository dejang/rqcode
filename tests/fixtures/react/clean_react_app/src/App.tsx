import React, { Suspense } from "react";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { OrderCard } from "./components/OrderCard";
import { Order } from "./models/order";
import { SettingsPage, routes } from "./routes";

const OrdersPage = React.lazy(() => import("./pages/Orders"));

export default () => {
  const order = new Order(120, "INV-100");
  order.add(30);
  const activeRouteCount = routes.length;

  return (
    <ErrorBoundary fallbackMessage="Unable to display workspace">
      <main data-routes={activeRouteCount}>
        <OrderCard order={order} />
        <Suspense fallback={<p>Loading</p>}>
          <OrdersPage />
          <SettingsPage />
        </Suspense>
      </main>
    </ErrorBoundary>
  );
};
