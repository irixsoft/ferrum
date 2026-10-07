import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/lib/theme";
import { RangeProvider } from "@/lib/range";
import { queryClient } from "@/lib/api";
import { router } from "@/router";
import "@/styles/index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <RangeProvider>
          <RouterProvider router={router} />
        </RangeProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </StrictMode>,
);
