import Nav from "./components/Nav";
import Hero from "./components/Hero";
import HowItWorks from "./components/HowItWorks";
import Features from "./components/Features";
import Privacy from "./components/Privacy";
import EngineTable from "./components/EngineTable";
import DownloadSection from "./components/Download";
import Faq from "./components/Faq";
import Footer from "./components/Footer";

export default function App() {
  return (
    <>
      <Nav />
      <main>
        <Hero />
        <HowItWorks />
        <Features />
        <Privacy />
        <EngineTable />
        <DownloadSection />
        <Faq />
      </main>
      <Footer />
    </>
  );
}
