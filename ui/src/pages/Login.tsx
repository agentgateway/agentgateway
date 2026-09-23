import { useEffect } from 'react';

import { apiBase } from '@/api/base';
import logoDark from '@/assets/agw-dark.svg';
import logoLight from '@/assets/agw-light.svg';
import { tr } from '@/i18n';

export function LoginPage() {
	const returnTo = new URLSearchParams(window.location.search).get('returnTo') || '/ui';
	const query = new URLSearchParams({ returnTo });

	useEffect(() => {
		document.title = tr('copy.signInPageTitle');
		document.documentElement.dataset.theme =
			localStorage.getItem('theme') ??
			(window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
	}, []);

	return (
		<main className="login-page">
			<section className="login-card" aria-labelledby="login-heading">
				<div className="login-brand">
					<img className="brand-logo brand-logo-light" src={logoLight} alt="agentgateway" />
					<img className="brand-logo brand-logo-dark" src={logoDark} alt="agentgateway" />
				</div>
				<h1 id="login-heading">{tr('copy.signIn')}</h1>
				<p>{tr('copy.signInWithIdentityProvider')}</p>
				<a className="button primary" href={`${apiBase}/api/auth/login?${query}`}>
					{tr('copy.signInWithSso')}
				</a>
			</section>
		</main>
	);
}
