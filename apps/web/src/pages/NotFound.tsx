import { ButtonLink, PageHeader } from "../components/ui";

export function NotFound() {
  return (
    <div>
      <PageHeader title="Page not found" subtitle="There is nothing at this address." />
      <ButtonLink to="/orders" variant="primary">
        Go to your orders
      </ButtonLink>
    </div>
  );
}
