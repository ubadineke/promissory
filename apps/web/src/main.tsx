import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Navigate, RouterProvider } from "react-router";
import { Layout } from "./components/Layout";
import { Allowances } from "./pages/Allowances";
import { InfoProvider } from "./info";
import { BatchPage } from "./pages/BatchPage";
import { Dashboard } from "./pages/Dashboard";
import { NewOrder } from "./pages/NewOrder";
import { NotFound } from "./pages/NotFound";
import { OrderPage } from "./pages/OrderPage";
import { PayeeStatus } from "./pages/PayeeStatus";
import { Payroll } from "./pages/Payroll";
import { Privacy } from "./pages/Privacy";
import { RequestPage, Requests } from "./pages/Requests";
import { ReceiptVerify } from "./pages/ReceiptVerify";
import { SubscribeBuilder } from "./pages/SubscribeBuilder";
import { SubscribeOpen } from "./pages/SubscribeOpen";
import { Switch } from "./pages/Switch";
import "./index.css";

// Only the landing page pulls in three.js; keep it out of the app bundle.
const Landing = lazy(() => import("./pages/Landing").then((m) => ({ default: m.Landing })));

const router = createBrowserRouter([
  {
    path: "/",
    element: (
      <Suspense fallback={null}>
        <Landing />
      </Suspense>
    ),
  },
  // The landing page's old address.
  { path: "/welcome", element: <Navigate to="/" replace /> },
  {
    element: <Layout />,
    children: [
      { path: "/orders", element: <Dashboard /> },
      { path: "/new", element: <NewOrder /> },
      { path: "/orders/:id", element: <OrderPage /> },
      { path: "/payroll", element: <Payroll /> },
      { path: "/payroll/:id", element: <BatchPage /> },
      { path: "/subscribe", element: <SubscribeBuilder /> },
      { path: "/s", element: <SubscribeOpen /> },
      { path: "/payee", element: <PayeeStatus /> },
      { path: "/privacy", element: <Privacy /> },
      { path: "/switch", element: <Switch /> },
      { path: "/receipt", element: <ReceiptVerify /> },
      { path: "/allowances", element: <Allowances /> },
      { path: "/requests", element: <Requests /> },
      { path: "/requests/:id", element: <RequestPage /> },
      { path: "*", element: <NotFound /> },
    ],
  },
]);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <InfoProvider>
      <RouterProvider router={router} />
    </InfoProvider>
  </StrictMode>,
);
