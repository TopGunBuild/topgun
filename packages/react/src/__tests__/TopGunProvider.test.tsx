import React from 'react';
import { render } from '@testing-library/react';
import { TopGunProvider, useClient } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';

// Mock TopGunClient
const createMockClient = (id?: string) => ({
  _id: id || 'test-client',
  query: jest.fn(),
  getMap: jest.fn(),
  getORMap: jest.fn(),
  topic: jest.fn(),
  getLock: jest.fn(),
  start: jest.fn(),
  setAuthToken: jest.fn(),
  setAuthTokenProvider: jest.fn(),
}) as unknown as TopGunClient;

describe('TopGunProvider', () => {
  it('should render children', () => {
    const mockClient = createMockClient();

    const { container } = render(
      <TopGunProvider client={mockClient}>
        <div data-testid="child">Hello World</div>
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="child"]')?.textContent).toBe('Hello World');
  });

  it('should render multiple children', () => {
    const mockClient = createMockClient();

    const { container } = render(
      <TopGunProvider client={mockClient}>
        <div data-testid="child1">First</div>
        <div data-testid="child2">Second</div>
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="child1"]')?.textContent).toBe('First');
    expect(container.querySelector('[data-testid="child2"]')?.textContent).toBe('Second');
  });

  it('should render nested components', () => {
    const mockClient = createMockClient();

    const NestedComponent = () => <span data-testid="nested">Nested</span>;

    const { container } = render(
      <TopGunProvider client={mockClient}>
        <div>
          <NestedComponent />
        </div>
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="nested"]')?.textContent).toBe('Nested');
  });

  it('should pass client through context', () => {
    const mockClient = createMockClient('my-client');

    const ClientConsumer = () => {
      const client = useClient();
      return <div data-testid="client-id">{(client as any)._id}</div>;
    };

    const { container } = render(
      <TopGunProvider client={mockClient}>
        <ClientConsumer />
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="client-id"]')?.textContent).toBe('my-client');
  });

  it('should provide client to deeply nested components', () => {
    const mockClient = createMockClient('deep-client');

    const DeepChild = () => {
      const client = useClient();
      return <span data-testid="deep">{(client as any)._id}</span>;
    };

    const MiddleComponent = ({ children }: { children: React.ReactNode }) => (
      <div>{children}</div>
    );

    const { container } = render(
      <TopGunProvider client={mockClient}>
        <MiddleComponent>
          <MiddleComponent>
            <MiddleComponent>
              <DeepChild />
            </MiddleComponent>
          </MiddleComponent>
        </MiddleComponent>
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="deep"]')?.textContent).toBe('deep-client');
  });

  it('should allow updating the client prop', () => {
    const client1 = createMockClient('client-1');
    const client2 = createMockClient('client-2');

    const ClientConsumer = () => {
      const client = useClient();
      return <div data-testid="client-id">{(client as any)._id}</div>;
    };

    const { container, rerender } = render(
      <TopGunProvider client={client1}>
        <ClientConsumer />
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="client-id"]')?.textContent).toBe('client-1');

    rerender(
      <TopGunProvider client={client2}>
        <ClientConsumer />
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="client-id"]')?.textContent).toBe('client-2');
  });

  it('should support nested providers with different clients', () => {
    const outerClient = createMockClient('outer');
    const innerClient = createMockClient('inner');

    const ClientConsumer = ({ testId }: { testId: string }) => {
      const client = useClient();
      return <div data-testid={testId}>{(client as any)._id}</div>;
    };

    const { container } = render(
      <TopGunProvider client={outerClient}>
        <ClientConsumer testId="outer-consumer" />
        <TopGunProvider client={innerClient}>
          <ClientConsumer testId="inner-consumer" />
        </TopGunProvider>
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="outer-consumer"]')?.textContent).toBe('outer');
    expect(container.querySelector('[data-testid="inner-consumer"]')?.textContent).toBe('inner');
  });

  it('should work with React fragments as children', () => {
    const mockClient = createMockClient();

    const { container } = render(
      <TopGunProvider client={mockClient}>
        <>
          <div data-testid="fragment-child-1">One</div>
          <div data-testid="fragment-child-2">Two</div>
        </>
      </TopGunProvider>
    );

    expect(container.querySelector('[data-testid="fragment-child-1"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="fragment-child-2"]')).not.toBeNull();
  });

  it('should handle empty children gracefully', () => {
    const mockClient = createMockClient();

    const { container } = render(
      <TopGunProvider client={mockClient}>
        {null}
      </TopGunProvider>
    );

    expect(container.childNodes.length).toBe(0);
  });

  it('should maintain client reference across re-renders', () => {
    const mockClient = createMockClient('stable-client');
    const clientRefs: TopGunClient[] = [];

    const ClientTracker = () => {
      const client = useClient();
      clientRefs.push(client);
      return <div>Tracking</div>;
    };

    const { rerender } = render(
      <TopGunProvider client={mockClient}>
        <ClientTracker />
      </TopGunProvider>
    );

    rerender(
      <TopGunProvider client={mockClient}>
        <ClientTracker />
      </TopGunProvider>
    );

    rerender(
      <TopGunProvider client={mockClient}>
        <ClientTracker />
      </TopGunProvider>
    );

    // All refs should be the same object
    expect(clientRefs[0]).toBe(clientRefs[1]);
    expect(clientRefs[1]).toBe(clientRefs[2]);
  });
});
