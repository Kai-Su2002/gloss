import type { ReactNode } from "react";

// Offline desktop builds deliberately have no Clerk account or sign-in controls.
export const useUser = () => ({ user: null, isSignedIn: false });
export const useAuth = () => ({ isLoaded: true, isSignedIn: false });
export const Show = (_props: { when: string; children: ReactNode }) => null;
export const SignInButton = (_props: { mode: string; children: ReactNode }) => null;
export const UserButton = (_props: unknown) => null;
