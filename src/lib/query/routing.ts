import { useQuery } from "@tanstack/react-query";
import { routingApi, type ActiveRoute } from "@/lib/api/routing";

/**
 * The route the proxy last actually used for `appType`.
 *
 * Polls while the proxy is taking over the app: the router records a decision
 * per request, so the value can change without any user interaction (failover,
 * circuit recovery). `null` data means "nothing routed yet".
 */
export function useActiveRoute(
  appType: string,
  options?: { enabled?: boolean },
) {
  return useQuery<ActiveRoute | null>({
    queryKey: ["activeRoute", appType],
    queryFn: () => routingApi.getActiveRoute(appType),
    enabled: (options?.enabled ?? true) && !!appType,
    refetchInterval: 5000,
    retry: false,
  });
}
