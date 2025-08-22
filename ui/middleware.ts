import { NextResponse } from 'next/server';
import type { NextRequest } from 'next/server';

// Redirect convenience: hitting the bare origin (/) goes to the UI basePath (/ui/)
export function middleware(req: NextRequest) {
  if (req.nextUrl.pathname === '/') {
    const url = req.nextUrl.clone();
    url.pathname = '/ui/';
    return NextResponse.redirect(url);
  }
  return NextResponse.next();
}

// Apply only at root
export const config = {
  matcher: ['/'],
};
