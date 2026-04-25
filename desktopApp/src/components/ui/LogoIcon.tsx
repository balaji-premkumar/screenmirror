import { FC } from "react";

const LogoIcon: FC = () => (
  <svg width="40" height="40" viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg">
    <rect x="2" y="2" width="36" height="36" rx="8" stroke="url(#logoGrad)" strokeWidth="2.5" fill="none"/>
    <path d="M12 14 L20 10 L28 14 L28 26 L20 30 L12 26 Z" fill="url(#logoGrad)" opacity="0.15"/>
    <path d="M12 14 L20 10 L28 14 M12 14 L20 18 L28 14 M20 18 L20 30" stroke="url(#logoGrad)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
    <defs>
      <linearGradient id="logoGrad" x1="0" y1="0" x2="40" y2="40" gradientUnits="userSpaceOnUse">
        <stop stopColor="#fb923c"/>
        <stop offset="1" stopColor="#ea580c"/>
      </linearGradient>
    </defs>
  </svg>
);

export default LogoIcon;