"use client";

import { useState, useEffect } from "react";

export interface UserClaim {
  typ: string;
  val: string;
}

export interface UserInfo {
  access_token?: string;
  expires_on?: string;
  id_token?: string;
  provider_name?: string;
  user_claims?: UserClaim[];
  user_id?: string;
}

export interface AuthUser {
  name: string;
  email: string;
  provider: "microsoft" | "google" | "unknown";
  isAuthenticated: boolean;
}

/**
 * Custom hook to fetch and manage Easy Auth user information
 * Fetches user data from Azure Container Apps Easy Auth /.auth/me endpoint
 */
export function useAuth() {
  const [user, setUser] = useState<AuthUser | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    async function fetchUserInfo() {
      try {
        const response = await fetch("/.auth/me");

        // If not authenticated, Easy Auth returns 401 or empty response
        if (!response.ok) {
          setUser(null);
          setIsLoading(false);
          return;
        }

        const data = await response.json();

        // Easy Auth returns an array of client principals
        if (!data || data.length === 0) {
          setUser(null);
          setIsLoading(false);
          return;
        }

        const userInfo: UserInfo = data[0];

        // Extract user information from claims
        const claims = userInfo.user_claims || [];
        const nameClaim = claims.find(
          (c) =>
            c.typ === "name" ||
            c.typ === "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/name" ||
            c.typ === "preferred_username"
        );
        const emailClaim = claims.find(
          (c) =>
            c.typ === "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress" ||
            c.typ === "email" ||
            c.typ === "emails"
        );

        // Determine provider from provider_name
        let provider: "microsoft" | "google" | "unknown" = "unknown";
        const providerName = userInfo.provider_name?.toLowerCase() || "";
        if (
          providerName.includes("aad") ||
          providerName.includes("microsoft") ||
          providerName.includes("azureactivedirectory")
        ) {
          provider = "microsoft";
        } else if (providerName.includes("google")) {
          provider = "google";
        }

        setUser({
          name: nameClaim?.val || "User",
          email: emailClaim?.val || "",
          provider,
          isAuthenticated: true,
        });
      } catch (err) {
        console.error("Error fetching auth user info:", err);
        setError(err instanceof Error ? err : new Error("Failed to fetch user info"));
        setUser(null);
      } finally {
        setIsLoading(false);
      }
    }

    fetchUserInfo();
  }, []);

  const logout = () => {
    window.location.href = "/.auth/logout";
  };

  return {
    user,
    isLoading,
    error,
    logout,
  };
}
