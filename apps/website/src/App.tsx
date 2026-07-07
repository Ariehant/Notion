import { Nav } from "./sections/Nav";
import { Hero } from "./sections/Hero";
import { Pillars } from "./sections/Pillars";
import { Security } from "./sections/Security";
import { Download } from "./sections/Download";
import { Footer } from "./sections/Footer";

export function App() {
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
