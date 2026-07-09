import { Nav } from "./sections/Nav";
import { Hero } from "./sections/Hero";
import { Pillars } from "./sections/Pillars";
import { Security } from "./sections/Security";
import { Download } from "./sections/Download";
import { Footer } from "./sections/Footer";
import { AndroidPage } from "./pages/AndroidPage";

/** True when the current path is the Android landing page (/android). */
function isAndroidRoute(): boolean {
  if (typeof window === "undefined") return false;
  return window.location.pathname.replace(/\/+$/, "").endsWith("/android");
}

export function App() {
  if (isAndroidRoute()) {
    return <AndroidPage />;
  }
  return (
    <>
      <Nav />
      <main>
        <Hero />
        <Pillars />
        <Security />
        <Download />
      </main>
      <Footer />
    </>
  );
}
